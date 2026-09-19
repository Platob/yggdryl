# Strings & bytes

One string family in eighteen real leaves, one byte family in six, the canonical text values - version, URL, time zone, MIME type, media type - and the regex that turns named captures into a schema.

## Contract

| | |
| --- | --- |
| Owns | `DataType::String(StringType)`, `DataType::Bytes(BytesType)`, the values `Str` and `Bytes`, `Version`, `Url`, `Timezone`, `MimeType`, `MediaType` |
| Constructors | `DataType::string` / `DataType::bytes` take the whole declaration; one constructor per leaf picks it once - `utf8`, `large_utf8`, `utf8_view`, `large_utf8_view`, `fixed_utf8(n)`, `sized_utf8(n)`, the same six as `ascii` and as `cp1252`, and `binary`, `large_binary`, `binary_view`, `large_binary_view`, `fixed_binary(n)`, `sized_binary(n)` |
| Reads back | `string_parameters`, `bytes_parameters`, `charset`, `fixed_byte_width`, `is_string`; a [code](codes.md), a [UUID](uuid.md) and a geospatial value answer no parameters, and a code answers `code_width` instead |
| Bound | the number *is* the leaf: the exact width on `fixed_*(n)`, the maximum stored bytes on `sized_*(n)`; neither stands without one, every other leaf refuses one, and zero is refused |
| Value | holds UTF-8 (or the payload) beside its leaf - the charset, the shape, and the width on a fixed leaf; never a maximum |
| Arrow | text storage for the UTF-8 and US-ASCII leaves, binary storage for the windows-1252 ones; `yggdryl.string` / `yggdryl.bytes` only where Arrow cannot say what is declared |
| Rust only | the value types `Str` and `Bytes`; the bindings read the parameters as frozen `StringParameters` / `BytesParameters` values |

### Strings

A string column is one of eighteen leaves: six shapes in each of the three
charsets that have a datatype - UTF-8, US-ASCII and windows-1252. The leaf is
the whole declaration. It says the [charset](../charset/index.md) its bytes
are written in, the shape Arrow lays them out in, and - on the two numbered
shapes - what its number means: `fixed_*(n)` is an exact width, NUL-padded,
and `sized_*(n)` a maximum, so neither stands without a number and the other
four refuse one. A leaf's canonical name is its `DataTypeId`, and its number
sits beside it in the table.

| shape | number | UTF-8 | US-ASCII | windows-1252 | Arrow storage (text / binary) |
| --- | --- | --- | --- | --- | --- |
| 32-bit offsets | none | `utf8` (27) | `ascii` (75) | `cp1252` (81) | `Utf8` / `Binary` |
| 64-bit offsets | none | `large_utf8` (30) | `large_ascii` (76) | `large_cp1252` (82) | `LargeUtf8` / `LargeBinary` |
| view | none | `utf8_view` (29) | `ascii_view` (77) | `cp1252_view` (83) | `Utf8View` / `BinaryView` |
| view, 64-bit offsets | none | `large_utf8_view` (31) | `large_ascii_view` (78) | `large_cp1252_view` (84) | `Utf8View` / `BinaryView` |
| fixed width | the exact width, required | `fixed_utf8(n)` (28) | `fixed_ascii(n)` (79) | `fixed_cp1252(n)` (85) | `FixedSizeBinary(n)` |
| bounded | the maximum, required | `sized_utf8(n)` (74) | `sized_ascii(n)` (80) | `sized_cp1252(n)` (86) | `Utf8` / `Binary` |

`utf8(32)` is `sized_utf8(32)` written short and `ascii(4)` is
`sized_ascii(4)`, because plain storage is exactly what a bounded column
fills; `large_utf8(64)` or `utf8_view(8)` would lose itself under a maximum,
so it says so rather than silently becoming something narrower. The
charset-free spellings - `string`, `fixed_string`, `string_view`,
`large_string`, `large_string_view`, `sized_string`, and the SQL `varchar`,
`char` and `text` - name the UTF-8 leaf of their shape, and they alone take a
charset in parentheses: `string(windows-1252,32)` is `sized_cp1252(32)`, and
`utf8(windows-1252)` is refused because the name already said. A charset with
no leaf - `string(iso-8859-1)` - is refused by name. Case, `_`, `-` and spaces
are ignored, so `large_utf8`, `largeutf8` and `LargeUtf8String` are one
datatype.

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `utf8` | unbounded UTF-8 | `string`, `str`, `text`, `varchar`, `nvarchar`, `char`, `character varying`, `utf8_string` |
| `sized_utf8(n)` | at most `n` bytes of UTF-8 | `utf8(n)`, `string(n)`, `varchar(n)`, `sized_string(n)` |
| `fixed_utf8(n)` | exactly `n` bytes of UTF-8 | `char(n)`, `character(n)`, `nchar(n)`, `fixed_string(n)` |
| `large_utf8`, `utf8_view`, `large_utf8_view` | unbounded UTF-8 in that shape | `large_string`, `string_view`, `large_string_view` |
| `ascii` | unbounded US-ASCII | `us_ascii`, `ascii_string`, `string(us-ascii)` |
| `sized_ascii(n)` | at most `n` US-ASCII bytes | `ascii(n)`, `string(us-ascii,n)` |
| `fixed_ascii(n)` | exactly `n` US-ASCII bytes | `fixed_string(us-ascii,n)` |
| `large_ascii`, `ascii_view`, `large_ascii_view` | unbounded US-ASCII in that shape | `large_string(us-ascii)`, `string_view(us-ascii)`, `large_string_view(us-ascii)` |
| `cp1252` | unbounded windows-1252 | `windows_1252`, `string(windows-1252)`, `string(cp1252)` |
| `sized_cp1252(n)` | at most `n` windows-1252 bytes | `cp1252(n)`, `string(windows-1252,n)` |
| `fixed_cp1252(n)` | exactly `n` windows-1252 bytes | `fixed_windows_1252(n)`, `fixed_string(windows-1252,n)` |
| `large_cp1252`, `cp1252_view`, `large_cp1252_view` | unbounded windows-1252 in that shape | `large_windows_1252`, `large_string(windows-1252)`, `string_view(cp1252)` |
| `version` | `Version` | - |
| `url` | `Url` | - |
| `timezone` | `Timezone` | `tz`, `timezone_name` |
| `mimetype` | `MimeType` | `mime` |
| `mediatype` | `MediaType` | `content_type` |

### Bytes

A byte column is one of six leaves, and the leaf is the whole declaration.
Four of them stand alone; the other two *are* a number - `fixed_binary(n)` is
an exact width and `sized_binary(n)` a maximum - so neither stands without one
and the four refuse one. Bytes are never padded, so a fixed value is exactly
its width.

`binary(n)` is `sized_binary(n)` written short, because plain binary is
exactly the storage a bounded column fills. Every other leaf would lose itself
under a maximum, so it says so rather than silently becoming something
narrower.

| leaf | spelling | number | also parsed as |
| --- | --- | --- | --- |
| 32-bit offsets | `binary` | none | `bytes`, `blob`, `bytea`, `varbinary` |
| 64-bit offsets | `large_binary` | none | - |
| view | `binary_view` | none | - |
| view, 64-bit offsets | `large_binary_view` | none | - |
| fixed width | `fixed_binary(n)` | the exact width, required | `fixed_size_binary(n)` |
| bounded | `sized_binary(n)` | the maximum, required | `binary(n)`, `varbinary(n)`, `varbinary_bounded(n)` |

Arrow has nowhere to put a maximum and one view width where this crate
declares two, so `sized_binary` and `large_binary_view` ride the
`yggdryl.bytes` document; the other four are Arrow's own.

## Use

Declare a string and a byte column, and read the declaration back.

=== "Rust"

    ```rust
    use yggdryl::{BytesType, StringType};
    use yggdryl::{Charset, DataType};

    // Every spelling of a leaf is one datatype, rendered under the leaf's
    // canonical name.
    assert_eq!(DataType::from_str("string")?, DataType::utf8());
    assert_eq!(DataType::from_str("varchar(32)")?.to_string(), "sized_utf8(32)");
    assert_eq!(DataType::from_str("char(8)")?, DataType::fixed_utf8(8)?);
    assert_eq!(DataType::from_str("fixed_string(us-ascii,4)")?, DataType::fixed_ascii(4)?);

    // A charset beside the general spelling picks that charset's leaf, and
    // the leaf reads back whole: charset, shape and number.
    let latin = DataType::from_str("string(windows-1252,32)")?;
    assert_eq!(latin, DataType::sized_cp1252(32)?);
    assert_eq!(latin.to_string(), "sized_cp1252(32)");
    let parameters = latin.string_parameters().expect("a string datatype");
    assert_eq!(parameters, StringType::SizedCp1252String(32));
    assert_eq!(parameters.charset(), Charset::Cp1252);
    assert_eq!(parameters.max(), Some(32));
    assert_eq!(parameters.storage(), StringType::Cp1252String);
    assert_eq!(latin.charset(), Some(Charset::Cp1252));

    // The number is the leaf: a maximum on a sized leaf, a width on a fixed one.
    assert_eq!(DataType::from_str("ascii(4)")?, DataType::sized_ascii(4)?);
    assert_eq!(DataType::fixed_ascii(4)?.fixed_byte_width(), Some(4));
    assert_eq!(DataType::ascii().fixed_byte_width(), None);
    // A large or view leaf holds no maximum, a charset-named spelling takes
    // no charset, and a charset with no leaf names nothing.
    assert!(DataType::from_str("large_utf8(64)").is_err());
    assert!(DataType::from_str("utf8(windows-1252)").is_err());
    assert!(DataType::from_str("string(iso-8859-1)").is_err());

    // Bytes: the leaf is the whole declaration, and a maximum is its own leaf.
    let bounded = DataType::from_str("varbinary(16)")?;
    assert_eq!(bounded.to_string(), "sized_binary(16)");
    assert_eq!(bounded.bytes_parameters(), Some(BytesType::SizedBinary(16)));
    assert_eq!(bounded.bytes_parameters().unwrap().max(), Some(16));
    assert_eq!(DataType::fixed_binary(16)?.fixed_byte_width(), Some(16));
    // A large binary is just a large binary; a maximum beside it is refused.
    assert!(DataType::from_str("large_binary(16)").is_err());
    assert!(DataType::binary().string_parameters().is_none());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import BytesParameters, DataType, StringParameters, types

    # Every spelling of a leaf is one datatype, rendered under the leaf's
    # canonical name.
    assert DataType("string") == DataType.utf8() == DataType("varchar")
    assert str(DataType("varchar(32)")) == "sized_utf8(32)"
    assert DataType("char(8)") == DataType.fixed_utf8(8)
    assert DataType("fixed_string(us-ascii,4)") == DataType.fixed_ascii(4)

    # A charset beside the general spelling picks that charset's leaf, and
    # it reads back as a frozen value naming the leaf.
    latin = DataType.string(charset="windows-1252", bound=32)
    assert str(latin) == "sized_cp1252(32)"
    assert latin == DataType("sized_cp1252(32)")
    assert latin.string_parameters == StringParameters("string", "windows-1252", 32)
    assert latin.string_parameters.layout == "sized_cp1252"
    assert latin.string_parameters.charset == "windows-1252"
    assert latin.string_parameters.max == 32
    assert latin.charset == "windows-1252"
    assert types.string("name", charset="windows-1252", max=32).dtype == latin
    assert types.sized_cp1252("name", 32).dtype == latin
    assert types.sized_cp1252("name", 32).dtype.id == "sized_cp1252"

    # The number is the leaf: a maximum on a sized leaf, a width on a fixed one.
    assert DataType("ascii(4)") == DataType("sized_ascii(4)")
    assert DataType("ascii(4)").string_parameters.max == 4
    assert DataType.fixed_ascii(4).fixed_byte_width == 4
    assert DataType.ascii().fixed_byte_width is None
    # A large or view leaf holds no maximum, a charset-named spelling takes
    # no charset, and a charset with no leaf names nothing.
    for refused in ("large_utf8(64)", "utf8(windows-1252)", "string(iso-8859-1)"):
        with pytest.raises(ValueError):
            DataType(refused)

    # Bytes: the leaf is the whole declaration, and a maximum is its own leaf.
    bounded = DataType("varbinary(16)")
    assert str(bounded) == "sized_binary(16)"
    assert bounded.bytes_parameters == BytesParameters("sized_binary", 16)
    assert bounded == DataType.bytes(bound=16)
    assert DataType.fixed_size_binary(16).fixed_byte_width == 16
    assert types.bytes("blob", layout="fixed_binary", fixed=16).dtype.id == "fixed_binary"
    assert DataType.binary().string_parameters is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    // Every spelling of a leaf is one datatype, rendered under the leaf's
    // canonical name.
    assert.ok(DataType.from('string').equals(DataType.utf8()))
    assert.equal(DataType.from('varchar(32)').toString(), 'sized_utf8(32)')
    assert.ok(DataType.from('char(8)').equals(DataType.fixedUtf8(8)))
    assert.ok(DataType.from('fixed_string(us-ascii,4)').equals(DataType.fixedAscii(4)))

    // A charset beside the general spelling picks that charset's leaf, and
    // it reads back as a plain object naming the leaf.
    const latin = DataType.string({ charset: 'windows-1252', max: 32 })
    assert.equal(latin.toString(), 'sized_cp1252(32)')
    assert.ok(latin.equals(DataType.from('sized_cp1252(32)')))
    assert.deepEqual(latin.stringParameters, {
      layout: 'sized_cp1252',
      charset: 'windows-1252',
      bound: 32,
      max: 32,
    })
    assert.equal(latin.charset, 'windows-1252')
    assert.ok(fields.string('name', { charset: 'windows-1252', max: 32 }).dtype.equals(latin))
    assert.ok(fields.sizedCp1252('name', 32).dtype.equals(latin))
    assert.equal(fields.sizedCp1252('name', 32).dtype.id, 'sized_cp1252')

    // The number is the leaf: a maximum on a sized leaf, a width on a fixed one.
    assert.ok(DataType.from('ascii(4)').equals(DataType.from('sized_ascii(4)')))
    assert.equal(DataType.from('ascii(4)').stringParameters.max, 4)
    assert.equal(DataType.fixedAscii(4).fixedByteWidth, 4)
    assert.equal(DataType.ascii().fixedByteWidth, null)
    // A large or view leaf holds no maximum, a charset-named spelling takes
    // no charset, and a charset with no leaf names nothing.
    for (const refused of ['large_utf8(64)', 'utf8(windows-1252)', 'string(iso-8859-1)']) {
      assert.throws(() => DataType.from(refused))
    }

    // Bytes: the leaf is the whole declaration, and a maximum is its own leaf.
    const bounded = DataType.from('varbinary(16)')
    assert.equal(bounded.toString(), 'sized_binary(16)')
    assert.deepEqual(bounded.bytesParameters, { layout: 'sized_binary', bound: 16, max: 16 })
    assert.ok(DataType.bytes({ max: 16 }).equals(bounded))
    assert.equal(DataType.fixedSizeBinary(16).fixedByteWidth, 16)
    assert.equal(fields.bytes('blob', { layout: 'fixed_binary', fixed: 16 }).dtype.id, 'fixed_binary')
    assert.equal(DataType.binary().stringParameters, null)
    ```

## Charsets and bounds

A bound counts **stored bytes**, not scalars: that is what the buffer holds and
what Arrow's offsets measure. Three charsets have leaves. UTF-8 and US-ASCII
are validated repertoires, so bytes that are not what they claim are refused
naming the charset, and a US-ASCII value holds no NUL and no byte above `0x7F`.
A windows-1252 leaf is a declaration that the column holds legacy bytes, and
those are *transcribed*: an unassigned byte reads as its ISO 8859-1 scalar,
which is what the WHATWG Encoding Standard's own index maps it to.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar};

    // Five scalars are five windows-1252 bytes and seven UTF-8 ones.
    assert_eq!(DataType::sized_cp1252(5)?.scalar("Grüße")?.as_str(), Some("Grüße"));
    assert!(DataType::sized_utf8(5)?.scalar("Grüße").is_err());

    // Bytes take one door, and the leaf's charset decides how strict it is.
    let latin = DataType::cp1252();
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
    assert DataType("sized_cp1252(5)").scalar("Grüße").as_py() == "Grüße"
    with pytest.raises(ValueError, match="at most 5 bytes"):
        DataType("sized_utf8(5)").scalar("Grüße")

    # Bytes take one door, and the leaf's charset decides how strict it is.
    latin = DataType("cp1252")
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
    const latin = fields.sizedCp1252('name', 5)
    assert.deepEqual(
      Array.from(latin.castArrowArray(text(['Grüße']), strict).get(0)),
      [0x47, 0x72, 0xfc, 0xdf, 0x65],
    )
    assert.throws(
      () => fields.sizedUtf8('name', 5).castArrowArray(text(['Grüße']), strict),
      /at most 5 bytes/,
    )

    // US-ASCII is a validated repertoire: no NUL, nothing above 0x7F.
    const ascii = fields.ascii('note')
    assert.deepEqual(Array.from(ascii.castArrowArray(text(['USD']), strict)), ['USD'])
    assert.throws(() => ascii.castArrowArray(text(['café']), strict), /non-ASCII/)
    assert.throws(() => ascii.castArrowArray(text(['a\0b']), strict), /NUL/)
    ```

A windows-1252 value may hold scalars the charset cannot write - that is what
recovering damage means - and the write seam (the Arrow array build, the cast)
refuses them naming the scalar.

## The values

`Str` and `Bytes` are the crate's string and byte string: a value up to
`INLINE_CAPACITY` (23) text bytes or `INLINE_BYTES` (30) payload bytes lives
inside the value with no heap behind it, a `'static` one costs nothing, and a
longer one is one shared `Arc` that clones by reference count. Equality, order
and hash read the characters or the payload alone, so a value is one value
whichever column holds it. A value remembers the leaf it is written under -
the charset, the shape, and the width on a fixed leaf - and never a maximum: a
cell read out of `sized_utf8(32)` is a `utf8`.

Rust only; the bindings read a value as a `Scalar` and its `dtype`.

```rust
use yggdryl::{Bytes, BytesType, INLINE_BYTES, INLINE_CAPACITY, Str, StringType};
use yggdryl::{DataType, Scalar};

// Short text lives inside the value; longer text is one shared handle.
let short = Str::new("AAPL");
assert!(short.is_inline());
assert!(!Str::new("a".repeat(INLINE_CAPACITY + 1)).is_inline());
assert_eq!(std::mem::size_of::<Str>(), 32);
assert_eq!(Scalar::from("AAPL"), Scalar::String(short.clone()));

// Restating a value under another leaf keeps the characters and changes what
// `encode` writes and `dtype` declares; a maximum is checked, not kept.
let latin = short.clone().try_with_parameters(StringType::LargeCp1252String)?;
assert_eq!(latin, short);
assert_eq!(latin.dtype()?, DataType::large_cp1252());
let bounded = short.clone().try_with_parameters(StringType::SizedUtf8String(8))?;
assert_eq!(bounded.parameters(), StringType::Utf8String);
assert!(short.clone().try_with_parameters(StringType::SizedUtf8String(2)).is_err());

// A fixed leaf pads on the way out and trims on the way in.
let ccy = Str::from_bytes(b"USD\0", StringType::FixedAsciiString(4))?;
assert_eq!(ccy, "USD");
assert_eq!(ccy.encode()?.as_ref(), b"USD\0");
assert_eq!(ccy.fixed(), Some(4));

// Bytes: the same holder over a payload, and a fixed width is exact.
let payload = Bytes::new([1_u8, 2, 3]);
assert!(payload.is_inline());
assert!(!Bytes::new(vec![0_u8; INLINE_BYTES + 1]).is_inline());
assert_eq!(std::mem::size_of::<Bytes>(), 40);
assert_eq!(Scalar::from(vec![1_u8, 2, 3]), Scalar::Bytes(payload.clone()));
let fixed = BytesType::FixedBinary(3);
assert_eq!(payload.clone().try_with_parameters(fixed)?.dtype()?, DataType::fixed_binary(3)?);
assert!(Bytes::new([1_u8, 2]).try_with_parameters(fixed).is_err());
```

The value door is the same rule in every language: a value read out of a
bounded column answers the plain leaf it fills.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let bounded = DataType::sized_ascii(4)?;
    assert_eq!(bounded.scalar("USD")?.dtype()?, DataType::ascii());
    assert!(bounded.scalar("EURO!").is_err());
    assert_eq!(DataType::from_str("binary(4)")?.scalar(vec![1_u8, 2, 3])?.dtype()?, DataType::binary());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    bounded = DataType("sized_ascii(4)")
    assert bounded.scalar("USD").dtype == DataType("ascii")
    with pytest.raises(ValueError, match="at most 4 bytes"):
        bounded.scalar("EURO!")
    assert DataType("binary(4)").scalar(b"\x01\x02\x03").dtype == DataType("binary")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields, json } = require('yggdryl')

    const bounded = fields.sizedAscii('code', 4)
    assert.equal(json.loads('"USD"', { field: bounded, scalar: true }).dtype.toString(), 'ascii')
    assert.throws(() => json.loads('"EURO!"', { field: bounded, scalar: true }), /at most 4 bytes/)
    ```

## Arrow storage

Arrow is told the truth about the bytes. The UTF-8 and US-ASCII leaves ride
Arrow's text layouts - ASCII bytes are UTF-8 - and the windows-1252 leaves the
matching *binary* layout; a fixed leaf rides `FixedSizeBinary` in every
charset. `utf8`, `large_utf8` and `utf8_view` are Arrow's own and cross bare;
every other leaf rides the `yggdryl.string` document, which states the leaf,
its charset - the one fact Arrow cannot - and its number:

| datatype | Arrow storage | extension name | document |
| --- | --- | --- | --- |
| `utf8`, `large_utf8`, `utf8_view` | `Utf8`, `LargeUtf8`, `Utf8View` | none | - |
| `sized_utf8(32)` | `Utf8` | `yggdryl.string` | `{"layout":"sized_utf8","charset":"utf-8","max":32}` |
| `ascii` | `Utf8` | `yggdryl.string` | `{"layout":"ascii","charset":"us-ascii"}` |
| `fixed_ascii(4)` | `FixedSizeBinary(4)` | `yggdryl.string` | `{"layout":"fixed_ascii","charset":"us-ascii","fixed":4}` |
| `large_utf8_view` | `Utf8View` | `yggdryl.string` | `{"layout":"large_utf8_view","charset":"utf-8"}` |
| `cp1252` | `Binary` | `yggdryl.string` | `{"layout":"cp1252","charset":"windows-1252"}` |
| `sized_cp1252(32)` | `Binary` | `yggdryl.string` | `{"layout":"sized_cp1252","charset":"windows-1252","max":32}` |
| `large_cp1252_view` | `BinaryView` | `yggdryl.string` | `{"layout":"large_cp1252_view","charset":"windows-1252"}` |
| `binary`, `large_binary`, `binary_view`, `fixed_binary(16)` | the same four | none | - |
| `sized_binary(16)` | `Binary` | `yggdryl.bytes` | `{"layout":"sized_binary","max":16}` |
| `large_binary_view` | `BinaryView` | `yggdryl.bytes` | `{"layout":"large_binary_view"}` |

A document over a storage it does not describe is a foreign field wearing our
name, and it imports as its storage. A code rides its own extension name
([Codes](codes.md)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    // Plain UTF-8 is Arrow's own datatype and crosses bare.
    let plain = Field::new("text", DataType::utf8(), true).into_arrow_field()?;
    assert_eq!(plain.data_type(), &ArrowDataType::Utf8);
    assert!(!plain.metadata().contains_key("ARROW:extension:name"));

    // US-ASCII is UTF-8, so it rides the text layout; the leaf rides the document.
    let note = Field::new("note", DataType::ascii(), false);
    let arrow = note.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.string");
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], r#"{"layout":"ascii","charset":"us-ascii"}"#);
    assert_eq!(Field::from_arrow_field(&arrow)?, note);

    // A fixed width is Arrow's fixed binary, whatever the charset.
    let ccy = Field::new("ccy", DataType::fixed_ascii(4)?, false);
    let arrow = ccy.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(4));
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], r#"{"layout":"fixed_ascii","charset":"us-ascii","fixed":4}"#);
    assert_eq!(Field::from_arrow_field(&arrow)?, ccy);

    // A windows-1252 leaf rides binary storage, because its bytes are not UTF-8.
    let latin = Field::new("name", DataType::sized_cp1252(32)?, true);
    let arrow = latin.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Binary);
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], r#"{"layout":"sized_cp1252","charset":"windows-1252","max":32}"#);
    assert_eq!(Field::from_arrow_field(&arrow)?, latin);

    // Bytes are the layout; only a maximum needs a document.
    assert!(!Field::new("blob", DataType::binary(), true).into_arrow_field()?.metadata().contains_key("ARROW:extension:name"));
    let capped = Field::new("blob", DataType::from_str("binary(16)")?, true).into_arrow_field()?;
    assert_eq!(capped.metadata()["ARROW:extension:name"], "yggdryl.bytes");
    assert_eq!(capped.metadata()["ARROW:extension:metadata"], r#"{"layout":"sized_binary","max":16}"#);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field, types

    # Plain UTF-8 is Arrow's own datatype and crosses bare.
    assert types.utf8("text").into_arrow().metadata is None

    # US-ASCII is UTF-8, so it rides the text layout; the leaf rides the document.
    note = types.ascii("note", nullable=False)
    arrow = note.into_arrow()
    assert arrow.type == pa.string()
    assert arrow.metadata == {
        b"ARROW:extension:name": b"yggdryl.string",
        b"ARROW:extension:metadata": b'{"layout":"ascii","charset":"us-ascii"}',
    }
    assert Field.from_arrow(arrow) == note

    # A fixed width is Arrow's fixed binary, whatever the charset.
    ccy = types.fixed_ascii("ccy", 4, nullable=False)
    arrow = ccy.into_arrow()
    assert arrow.type == pa.binary(4)
    assert arrow.metadata[b"ARROW:extension:metadata"] == (
        b'{"layout":"fixed_ascii","charset":"us-ascii","fixed":4}'
    )
    assert Field.from_arrow(arrow) == ccy
    assert Field.from_arrow(pa.field("ccy", pa.binary(4))) == Field("ccy", "fixed_binary(4)")

    # A windows-1252 leaf rides binary storage, because its bytes are not UTF-8.
    latin = types.sized_cp1252("name", 32)
    arrow = latin.into_arrow()
    assert arrow.type == pa.binary()
    assert arrow.metadata[b"ARROW:extension:metadata"] == (
        b'{"layout":"sized_cp1252","charset":"windows-1252","max":32}'
    )
    assert Field.from_arrow(arrow) == latin

    # Bytes are the layout; only a maximum needs a document.
    assert types.binary("blob").into_arrow().metadata is None
    capped = Field("blob", "binary(16)").into_arrow()
    assert capped.metadata == {
        b"ARROW:extension:name": b"yggdryl.bytes",
        b"ARROW:extension:metadata": b'{"layout":"sized_binary","max":16}',
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

    // US-ASCII is UTF-8, so it rides the text layout; the leaf rides the document.
    const note = projected(fields.ascii('note', { nullable: false }))
    assert.equal(String(note.type), 'Utf8')
    assert.equal(note.metadata.get('ARROW:extension:name'), 'yggdryl.string')
    assert.equal(
      note.metadata.get('ARROW:extension:metadata'),
      '{"layout":"ascii","charset":"us-ascii"}',
    )

    // A fixed width is Arrow's fixed binary, whatever the charset.
    const ccy = projected(fields.fixedAscii('ccy', 4, { nullable: false }))
    assert.equal(String(ccy.type), 'FixedSizeBinary[4]')
    assert.equal(
      ccy.metadata.get('ARROW:extension:metadata'),
      '{"layout":"fixed_ascii","charset":"us-ascii","fixed":4}',
    )

    // A windows-1252 leaf rides binary storage, because its bytes are not UTF-8.
    const latin = projected(fields.sizedCp1252('name', 32))
    assert.equal(String(latin.type), 'Binary')
    assert.equal(
      latin.metadata.get('ARROW:extension:metadata'),
      '{"layout":"sized_cp1252","charset":"windows-1252","max":32}',
    )

    // Bytes are the layout; only a maximum needs a document.
    assert.equal(projected(fields.binary('blob')).metadata.get('ARROW:extension:name'), undefined)
    const capped = projected(fields.bytes('blob', { max: 16 }))
    assert.equal(capped.metadata.get('ARROW:extension:name'), 'yggdryl.bytes')
    assert.equal(capped.metadata.get('ARROW:extension:metadata'), '{"layout":"sized_binary","max":16}')
    ```

## Casts

A string target on any leaf but the three Arrow's own - a maximum, a fixed
width, a charset other than UTF-8 - validates every cell on the way in
(`StringIngest`); a bounded variable byte target checks every cell's length
(`BytesIngest`). `utf8`, `large_utf8`, `utf8_view` and the four plain byte
leaves stay Arrow's own kernel. A source with a `yggdryl.string` document is
read under its own leaf and restated under the target's; a code source is read
as its trimmed text; bare text storage is read as text; bare binary storage is
read as bytes already in the target charset (a fixed source trimmed of NUL
first). Under `safe` a failing cell becomes null, under strict an error names
the row and the column. A fixed leaf pads on the way in, and the stored column
read back under `utf8` trims.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, StringArray};
    use yggdryl::FieldValue as _;
use yggdryl::{ArrowCastOptions, DataType, Field};

    let strict = ArrowCastOptions::new().with_safe(false);
    let ccy = Field::new("ccy", DataType::fixed_ascii(4)?, true);

    // A cast into the width pads.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EU"]));
    let padded = ccy.cast_arrow_array(text, strict)?;
    let cells = padded.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(cells.value(1), b"EU\0\0");

    // A stored column carrying the document reads back under `utf8` trimmed.
    let stored = ccy.clone().into_arrow_field()?;
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
with `layout` naming the leaf (omitted when `utf8` / `binary`) and `fixed` or
`max` beside it (omitted when the leaf carries no number). The leaf names its
charset, so no `charset` key is written; a document carrying one restates the
leaf in that charset's family, so `{"type":"string","layout":"large_string","charset":"windows-1252"}`
reads as `large_cp1252`. A scalar crosses as `{"type":"string","value":...}`
or `{"type":"bytes","value":...}`; the value is bare text or bytes under the
default leaf and an object naming the leaf otherwise.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    assert_eq!(DataType::utf8().into_json()?, r#"{"type":"string"}"#);
    assert_eq!(DataType::from_str("utf8(32)")?.into_json()?, r#"{"type":"string","layout":"sized_utf8","max":32}"#);
    assert_eq!(
        DataType::fixed_ascii(4)?.into_json()?,
        r#"{"type":"string","layout":"fixed_ascii","fixed":4}"#
    );
    assert_eq!(
        DataType::from_json(r#"{"type":"string","layout":"large_string","charset":"windows-1252"}"#)?,
        DataType::large_cp1252()
    );
    assert_eq!(
        DataType::from_str("binary(16)")?.into_json()?,
        r#"{"type":"binary","layout":"sized_binary","max":16}"#
    );
    assert_eq!(
        DataType::fixed_binary(16)?.into_json()?,
        r#"{"type":"binary","layout":"fixed_binary","fixed":16}"#
    );
    let field = Field::new("name", DataType::fixed_cp1252(8)?, true);
    assert_eq!(Field::from_json(&field.clone().into_json()?)?, field);
    // The retired tags are not read back.
    assert!(DataType::from_json(r#"{"type":"utf8"}"#).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    assert DataType("utf8").into_dict() == {"type": "string"}
    assert DataType("utf8(32)").into_dict() == {"type": "string", "layout": "sized_utf8", "max": 32}
    assert DataType.fixed_ascii(4).into_dict() == {
        "type": "string",
        "layout": "fixed_ascii",
        "fixed": 4,
    }
    assert DataType.from_json(
        '{"type":"string","layout":"large_string","charset":"windows-1252"}'
    ) == DataType("large_cp1252")
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
    field = Field("name", "fixed_cp1252(8)")
    assert Field.from_json(field.into_json()) == field
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    assert.deepEqual(DataType.utf8().toJSON(), { type: 'string' })
    assert.deepEqual(DataType.from('utf8(32)').toJSON(), { type: 'string', layout: 'sized_utf8', max: 32 })
    assert.deepEqual(DataType.fixedAscii(4).toJSON(), {
      type: 'string',
      layout: 'fixed_ascii',
      fixed: 4,
    })
    assert.ok(
      DataType.fromJSON({ type: 'string', layout: 'large_string', charset: 'windows-1252' }).equals(
        DataType.from('large_cp1252'),
      ),
    )
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
    const field = new Field('name', 'fixed_cp1252(8)', true)
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
| Fraction sign | either decimal sign ISO 8601 names, `.` or `,` |
| Fraction width | a capture admitting several widths takes the widest spelling it matches, the only resolution that holds every row it admits |
| Broad captures | a capture such as `\S+` stays `utf8` |
| Rows read | none, so [plain-text records](../media/text/index.md) publish a schema before opening a source |

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

## Time zones

`Timezone` is the crate's own zone carried as a column: the same four-byte
interned handle every temporal datatype and value already declares, so a zone
read out of a table is a zone a `datetime64` column can be built with, not text
that happens to name one. Parsing canonicalizes, so an alias, a case and a
fixed offset each hold one spelling.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, TimeUnit, Timezone};

    let zone = Timezone::from_str("Asia/Calcutta")?;
    // An alias resolves to what it stands for, so two spellings are one value.
    assert_eq!(zone.as_str(), "Asia/Kolkata");
    assert_eq!(Timezone::from_str("-0800")?.as_str(), "-08:00");

    let field = Field::new("zone", DataType::Timezone, false);
    assert_eq!(field.scalar("Asia/Calcutta")?, Scalar::Timezone(zone));
    // The value is the one a temporal column declares, rules and all.
    assert_eq!(Scalar::datetime64(0, TimeUnit::Second, zone)?.temporal_timezone(), Some(zone));
    assert!(field.scalar("+99:00").is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar, types
    from yggdryl.text import json

    dtype = DataType("timezone")
    field = types.timezone("zone", nullable=False)
    value = json.loads('"Asia/Calcutta"', field=field, cls=Scalar)
    assert dtype.kind == "text"
    assert value.as_py() == "Asia/Kolkata"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields, json } = require('yggdryl')

    const dtype = new DataType('timezone')
    const value = json.loads('"Asia/Calcutta"', {
      field: fields.timezone('zone', { nullable: false }),
      scalar: true,
    })
    assert.equal(dtype.kind, 'text')
    assert.equal(value.asJs(), 'Asia/Kolkata')
    ```

| rule | behaviour |
| --- | --- |
| Kind | `text`; the aliases are `TimezoneField`, `types.timezone`, `fields.timezone` |
| Value | `crate::Timezone`, four bytes, interned for the process lifetime |
| Storage | `Utf8` holding the canonical name, extension name `yggdryl.timezone` |
| Ordering | the canonical name's, which is Arrow's own string order |
| Default | `NAIVE`, the zone-free marker every temporal already defaults to |
| Merging | only with itself: merging into text would drop the canonicalization |
| Bindings | the value crosses as its canonical name; a zone has no wrapper class of its own in either language |

## Media types

`MimeType` is one canonical `type/subtype` name; `MediaType` is that name with
the [charset](../charset/index.md) and the ordered content codings it was
declared under. Both are the values the [media layer](../media/index.md) routes
a record read on, carried as columns rather than restated.

A MIME type refuses text that is not a `type/subtype` name. A media type does
not: it is also the crate's filename and content-negotiation reader, so its
intake is total by construction and unrecognized text answers the default base.

=== "Rust"

    ```rust
    use yggdryl::{Charset, DataType, Field, MediaType, MimeType, Scalar};

    let mime = MimeType::from_str("APPLICATION/JSON")?;
    assert_eq!(mime, MimeType::JSON);
    assert_eq!(mime.as_str(), "application/json");

    let field = Field::new("held", DataType::MimeType, false);
    assert_eq!(field.scalar("APPLICATION/JSON")?, Scalar::MimeType(mime));
    assert!(field.scalar("not a type").is_err());

    // A media type carries the charset and the codings in one rendering.
    let media = MediaType::from_str("text/csv; charset=utf-8")?;
    assert_eq!(media.base(), &MimeType::CSV);
    assert_eq!(media.charset(), Some(Charset::Utf8));
    let field = Field::new("held", DataType::MediaType, false);
    assert_eq!(field.scalar("TEXT/CSV; CHARSET=UTF-8")?, Scalar::from(media));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar, types
    from yggdryl.text import json

    field = types.mimetype("held", nullable=False)
    value = json.loads('"APPLICATION/JSON"', field=field, cls=Scalar)
    assert DataType("mimetype").kind == "text"
    assert value.as_py() == "application/json"

    media = types.mediatype("held", nullable=False)
    declared = json.loads('"TEXT/CSV; CHARSET=UTF-8"', field=media, cls=Scalar)
    assert declared.as_py() == "text/csv;charset=utf-8"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields, json } = require('yggdryl')

    const value = json.loads('"APPLICATION/JSON"', {
      field: fields.mimetype('held', { nullable: false }),
      scalar: true,
    })
    assert.equal(new DataType('mimetype').kind, 'text')
    assert.equal(value.asJs(), 'application/json')

    const declared = json.loads('"TEXT/CSV; CHARSET=UTF-8"', {
      field: fields.mediatype('held', { nullable: false }),
      scalar: true,
    })
    assert.equal(declared.asJs(), 'text/csv;charset=utf-8')
    ```

| rule | behaviour |
| --- | --- |
| Kind | `text`; the aliases are `MimeTypeField` and `MediaTypeField`, `types.mimetype` / `fields.mimetype` and `types.mediatype` / `fields.mediatype` |
| Value | `crate::MimeType` inline; `crate::MediaType` behind one shared pointer, because a base, a charset and a coding list are wider than the scalar |
| Storage | `Utf8` holding the canonical text, extension names `yggdryl.mimetype` and `yggdryl.mediatype` |
| Intake | a MIME type refuses a name that is not `type/subtype`; a media type infers, so every text has an answer |
| Default | `application/octet-stream`, which is what both values answer `Default` with |
| Merging | only with itself, and never with each other: a media type models a charset and codings a MIME type does not |
| Bindings | both cross as their canonical text; neither has a wrapper class of its own in Python or JavaScript |

## Edges

- `fixed_utf8`, `sized_ascii`, `fixed_binary` with no number -> refused; the number is what makes the leaf. A bound of `0` -> refused, `at least one byte, got 0`.
- `utf8(windows-1252)`, `ascii(windows-1252)` -> refused; a charset-named spelling declares its charset in the name. `string(iso-8859-1)` -> refused; only UTF-8, US-ASCII and windows-1252 have a leaf.
- `utf8(32)`, `ascii(4)`, `cp1252(32)`, `binary(16)` -> the sized leaf written short; `large_utf8(64)`, `utf8_view(8)`, `large_binary(16)` -> refused, a large or view leaf holds no maximum. `varchar(255)` -> `sized_utf8(255)`; `char(8)` -> `fixed_utf8(8)`; bare `char` -> `utf8`.
- A bound counts stored bytes, so `sized_utf8(4)` refuses `Grüß` and `sized_cp1252(4)` holds it.
- Every UTF-8 and US-ASCII leaf reads bytes strictly, naming the charset; every windows-1252 leaf transcribes, an unassigned byte as its ISO 8859-1 scalar.
- A US-ASCII value -> no NUL, no byte above `0x7F`, refused naming the byte and its position; a variable US-ASCII value keeps its length, only a fixed leaf trims trailing NUL.
- A fixed string stores its value padded with trailing NUL and reads back trimmed; a fixed byte value is exactly its width, never padded.
- Text windows-1252 has no bytes for -> held as a value, refused when the column is written, naming the scalar; the value door counts rather than judges.
- A value never carries a maximum: `Scalar::dtype()` of a cell read out of `sized_utf8(32)` is `utf8`, of `sized_binary(16)` is `binary`.
- `Scalar::from("USD")` and a value read out of an `ascii` column are one value; `Str` equality, order and hash read the characters alone.
- `string_parameters` on a code, `bytes_parameters` on a UUID -> `None`; `fixed_byte_width` answers for a fixed string, fixed bytes, a UUID and the numbers, and a code answers `code_width`, the maximum its standard fixes over the text it stores.
- A `yggdryl.string` or `yggdryl.bytes` document over a storage it does not describe -> imports as the storage.
- A stored column carrying `yggdryl.msgdirection` or `yggdryl.direction` -> imports as the `fixed_size_binary(4)` it is: the datatype was retired, and which way a message moved is FIX's tag 385, text over its code set.
- Arrow JS rows carry no extension identity, so a `fixed_ascii(n)` column arrives as its padded bytes through `readRecords`; declare `utf8` to read text.
- A `StringIngest` or `BytesIngest` refusal under `safe` -> null, which a required column then fills with the default; under strict -> `field "<name>" row <n>: expected ..., got ...`.
- Avro and Iceberg -> a UTF-8 or US-ASCII leaf crosses as `string`, a fixed one trimmed of padding; a windows-1252 leaf is refused by name; a bounded byte column crosses unbounded, the bound enforced where values enter.
- Merging follows [Field](field.md): two strings and two byte types meet parameter by parameter.
- `from_regex(pattern, false)` -> every capture stays `utf8`; invalid regex syntax or an expression past the recursion limit -> datatype error.
- `\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}` -> a millisecond datetime and `\d{2}:\d{2}:\d{2},\d{3}` -> a millisecond time: the comma is a decimal sign inside a clock. Outside one it is not, so `\d+,\d+` and `\d{1,3}(?:,\d{3})*` stay `utf8` and a bare fraction such as `,\d{3}` carries no clock to be part of.
- A capture admitting several widths takes the widest: `\.\d{1,5}` -> microseconds, and an optional or variable fraction publishes the widest unit it admits even where every row spells none, so `\d{2}:\d{2}:\d{2}(?:\.\d{2})?` -> `time32(ms)`. A capture spelling one width it once had no candidate for - two, four, seven or eight digits - is now that width's datetime rather than `utf8`, and a row the reader refuses in such a column is null under `safe` rather than the text it used to stay.
- `005.0.000` -> the canonical `5`; a version whose major or minor exceeds `255`, or whose major is not a decimal number -> refused at the first bad byte; a fourth component, an empty component, a qualifier, or a patch above `65535` -> folded into the patch.
- Fractional or out-of-range constructor arguments in Python or JavaScript -> refused without narrowing.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::string datatype::bytes datatype::ascii field::ascii field::binary strings:: bytes:: regex:: version::
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- string::tests version::tests
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

### Allocations per row

A `Str` holds its first twenty-three bytes inline and shares an `Arc<str>`
above them; a `Bytes` holds thirty inline and shares an `Arc<[u8]>` above
them. Those thresholds are the whole allocation story: below one a cell is
free to build and free to clone, above it it is one shared handle and one
copy. These counts are measured with the counting allocator and asserted in
`rust/tests/allocations.rs` - the two inline thresholds, a column built at
sixteen, a thousand and sixteen thousand rows, and a cell transcoded on each
side of the buffer - not timed: a count is the same on every machine, and a
timing is not. A cell read out of Arrow takes the same door a value does, so
the `read` rows are the value's cost.

| path | cell within the inline buffer | cell past it |
| --- | ---: | ---: |
| `utf8` / `ascii` build | one buffer per column | one buffer per column |
| `utf8` / `ascii` read | 0 | 1 |
| `cp1252` build | one buffer per column | one buffer per column |
| `cp1252` read, all-ASCII cell | 0 | 1 |
| `cp1252` read, transcoded cell | 0 | 2 |
| `binary` read | 0 | 1 |

A column's build cost is its buffers and not its rows: the payload is measured
with [`Charset::encoded_len`](../charset/index.md) before a byte of it is
built, so the count is equal at sixteen rows and at sixteen thousand. The
`read` row past the inline buffer is one handle per cell out of a buffer Arrow
already shares; removing it needs a storage handle that does not fit
[`Scalar`](scalar.md)'s pinned forty-eight bytes, so it is recorded rather
than spent.

A transcoded cell past the buffer costs two because the text is built once and
copied once into the shared handle, and `String` and `Arc<str>` have different
layouts, so no conversion between them is free.

### Timings

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
| `sized_cp1252(32)` datatype | 543.1 ns/op | 223,772 ops/s |
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
