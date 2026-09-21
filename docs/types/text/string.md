# String

One string datatype in eighteen real leaves: six shapes in each of the three charsets that have one, the value `Str` they hold, and everything a string column declares.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::String(StringType)`, the eighteen leaves, the value `Str`, the `FIELD:enum` dictionary `StringEnum` and its ISO listings, one Arrow projection, one cast tier, one grammar |
| Validates | The number at construction, the charset and the bound at the value door; a US-ASCII value holds no NUL and no byte above `0x7F` |
| Lazy | Nothing - a leaf is a copy value, and a `Str` decodes at the seam rather than on read |
| Cached | The Arrow projection of a [`Field`](../field.md); a value up to `INLINE_CAPACITY` bytes lives inside the `Str` itself |
| Refuses | A charset with no leaf, a charset beside a charset-named spelling, a number beside an unnumbered leaf, a numbered leaf with no number, a bound of zero, and bytes that are not the charset they claim |
| Kinds | `DataTypeKind::Text`, ids `0x51`-`0x62`: one `DataTypeId` per leaf, laid out by family |
| Bindings | `Str` is Rust only; Python and JavaScript read a value as a [`Scalar`](../scalar.md) and the declaration as the frozen `StringParameters` |

The twelve [registered codes](../codes/index.md) are not strings: a currency is
an identity over ISO 4217 that stores as the text it is, so it is
`DataType::Currency`, kind `Code`, answers `code_width`, and never
`string_parameters`.

## DataType

`DataType::string` takes the whole declaration - a layout, a charset and a
number - and each leaf constructor is that call with the leaf picked once.
Every spelling of a leaf is one datatype and renders under the leaf's canonical
name, which is its `DataTypeId`. The charset-free spellings - `string`,
`fixed_string`, `string_view`, `large_string`, `large_string_view`,
`sized_string`, and the SQL `varchar`, `char` and `text` - name the UTF-8 leaf
of their shape, and they alone take a charset in parentheses:
`string(windows-1252,32)` is `sized_cp1252(32)`, while `utf8(windows-1252)` is
refused because the name already said. A charset with no leaf -
`string(iso-8859-1)` - is refused by name. The grammar's fold drops case,
underscores, hyphens and spaces, so `large_utf8`, `largeutf8` and
`LARGE-UTF8` are one datatype.

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

=== "Rust"

    ```rust
    use yggdryl::{Charset, DataType, DataTypeId, StringType};

    // Every spelling of a leaf is one datatype, rendered under the leaf's
    // canonical name.
    assert_eq!(DataType::from_str("string")?, DataType::utf8());
    assert_eq!(DataType::from_str("varchar(32)")?.to_string(), "sized_utf8(32)");
    assert_eq!(DataType::from_str("char(8)")?, DataType::fixed_utf8(8)?);
    assert_eq!(DataType::from_str("fixed_string(us-ascii,4)")?, DataType::fixed_ascii(4)?);
    assert_eq!(DataType::from_str("LARGE-UTF8")?, DataType::large_utf8());

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

    // The leaf is the identifier, and the identifier is the wire contract.
    assert_eq!(DataType::utf8().id(), DataTypeId::Utf8String);
    assert_eq!(DataTypeId::Utf8String.as_u8(), 0x51);
    assert_eq!(latin.id(), DataTypeId::SizedCp1252String);
    assert_eq!(DataTypeId::SizedCp1252String.as_u8(), 0x62);
    assert_eq!(StringType::ALL.len(), 18);

    // The number is the leaf: a maximum on a sized leaf, a width on a fixed one.
    assert_eq!(DataType::from_str("ascii(4)")?, DataType::sized_ascii(4)?);
    assert_eq!(DataType::fixed_ascii(4)?.fixed_byte_width(), Some(4));
    assert_eq!(DataType::ascii().fixed_byte_width(), None);
    // A large or view leaf holds no maximum, a charset-named spelling takes
    // no charset, and a charset with no leaf names nothing.
    assert!(DataType::from_str("large_utf8(64)").is_err());
    assert!(DataType::from_str("utf8(windows-1252)").is_err());
    assert!(DataType::from_str("string(iso-8859-1)").is_err());
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    from yggdryl import DataType, StringParameters

    # Every spelling of a leaf is one datatype, rendered under the leaf's
    # canonical name.
    assert DataType("string") == DataType.utf8() == DataType("varchar")
    assert str(DataType("varchar(32)")) == "sized_utf8(32)"
    assert DataType("char(8)") == DataType.fixed_utf8(8)
    assert DataType("fixed_string(us-ascii,4)") == DataType.fixed_ascii(4)
    assert DataType("LARGE-UTF8") == DataType("large_utf8")

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
    assert yggdryl.string("name", charset="windows-1252", max=32).dtype == latin
    assert yggdryl.sized_cp1252("name", 32).dtype == latin

    # The leaf is the identifier.
    assert DataType("utf8").id == "utf8"
    assert yggdryl.sized_cp1252("name", 32).dtype.id == "sized_cp1252"

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
    assert.ok(DataType.from('LARGE-UTF8').equals(DataType.from('large_utf8')))

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
    ```

## Field

`StringField` is the typed marker: one field carrying `StringType` itself, so
the leaf is read off the payload rather than matched out of a root datatype.
`new` takes the payload, `try_new` a root `DataType` and refuses another family
by name. The bindings have one factory per leaf plus `types.string` /
`fields.string` for the whole declaration, nullable unless the call says
otherwise; metadata rides beside the datatype, never inside it. A column of
locations or names is not a string at all - it declares `url` or `urn`
([URI](../../uri/index.md#as-a-column)) - and the reserved metadata keys a
field carries are on [Protocol](../protocol.md).

=== "Rust"

    ```rust
    use yggdryl::{Charset, DataType, DataTypeId, Field, StringField, StringType};

    let name = StringField::new("name", StringType::SizedCp1252String(32), false);
    assert_eq!(name.name(), "name");
    assert_eq!(name.dtype(), &DataType::sized_cp1252(32)?);
    assert_eq!(name.typed_dtype_ref().charset(), Charset::Cp1252);
    assert_eq!(name.id(), DataTypeId::SizedCp1252String);
    assert!(!name.is_nullable());

    // Widened, it is the same column the root constructor builds.
    assert_eq!(name.to_field(), Field::new("name", DataType::sized_cp1252(32)?, false));
    // A datatype from another family is refused by name.
    let refused = StringField::try_new("name", DataType::binary(), false)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("string"), "{refused}");

    // Metadata rides beside the datatype.
    let note = Field::from_parts("note", DataType::utf8(), true, [("source", "feed")])?;
    assert_eq!(note.get_metadata("source"), Some("feed"));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    name = yggdryl.sized_cp1252("name", 32, nullable=False)
    assert isinstance(name, Field)
    assert name.name == "name"
    assert str(name.dtype) == "sized_cp1252(32)"
    assert name.nullable is False

    # One factory per leaf, and one that takes the whole declaration.
    assert yggdryl.string("name", charset="windows-1252", max=32).dtype == name.dtype
    assert yggdryl.fixed_ascii("ccy", 4).dtype == yggdryl.string("ccy", layout="fixed_ascii", fixed=4).dtype

    # Metadata rides beside the datatype, never inside it.
    note = yggdryl.utf8("note", metadata={"source": "feed"})
    assert note.metadata["source"] == "feed"
    assert note.nullable is True
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const name = fields.sizedCp1252('name', 32, { nullable: false })
    assert.ok(name instanceof Field)
    assert.equal(name.name, 'name')
    assert.equal(name.dtype.toString(), 'sized_cp1252(32)')
    assert.equal(name.nullable, false)

    // One factory per leaf, and one that takes the whole declaration.
    assert.ok(fields.string('name', { charset: 'windows-1252', max: 32 }).dtype.equals(name.dtype))
    assert.ok(fields.fixedAscii('ccy', 4).dtype.equals(fields.string('ccy', { layout: 'fixed_ascii', fixed: 4 }).dtype))

    // Metadata rides beside the datatype, never inside it.
    const note = fields.utf8('note', { metadata: { source: 'feed' } })
    assert.equal(note.nullable, true)
    assert.equal(note.get('source'), 'feed')
    ```

## Scalar

`Scalar::String(Str)` is the value: the characters, beside the leaf they are
stored under. A maximum is the column's rule and never the value's, so
`storage()` is what a value in a sized column carries - the plain leaf of its
charset - and a cell read out of `sized_utf8(32)` is a `utf8`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, Str, StringType};

    // The value door checks the charset and the bound, and answers the plain
    // leaf of its charset rather than the column's maximum.
    let bounded = DataType::sized_ascii(4)?;
    let value = bounded.scalar("USD")?;
    assert_eq!(value.as_str(), Some("USD"));
    assert_eq!(value.dtype()?, DataType::ascii());
    assert!(bounded.scalar("EURO!").is_err());

    // `Str` is the holder every string-family API answers with.
    assert_eq!(Scalar::from("AAPL"), Scalar::String(Str::new("AAPL")));
    assert_eq!(Str::new("AAPL").parameters(), StringType::Utf8String);
    assert_eq!(Str::new("AAPL").as_str(), "AAPL");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    bounded = DataType("sized_ascii(4)")
    value = bounded.scalar("USD")
    assert value.as_py() == "USD"
    assert value.dtype == DataType("ascii")
    assert value.kind == "ascii"
    assert value.family == "text"
    with pytest.raises(ValueError, match="at most 4 bytes"):
        bounded.scalar("EURO!")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const bounded = DataType.from('sized_ascii(4)')
    const value = bounded.scalar('USD')
    assert.equal(value.asJs(), 'USD')
    assert.equal(value.dtype.toString(), 'ascii')
    assert.equal(value.kind, 'ascii')
    assert.equal(value.family, 'text')
    assert.throws(() => bounded.scalar('EURO!'), /at most 4 bytes/)
    ```

`Str` itself is Rust only: a value up to `INLINE_CAPACITY` (23) text bytes
lives inside it with no heap behind it, a `'static` one costs nothing, and a
longer one is one shared `Arc` that clones by reference count. Equality, order
and hash read the characters alone, so a value is one value whichever column
holds it, and `as_str` is infallible on every string value there is.

```rust
use yggdryl::{DataType, INLINE_CAPACITY, Str, StringType};

// Short text lives inside the value; longer text is one shared handle.
let short = Str::new("AAPL");
assert!(short.is_inline());
assert!(!Str::new("a".repeat(INLINE_CAPACITY + 1)).is_inline());
assert_eq!(std::mem::size_of::<Str>(), 32);

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
```

## Arrow storage

Arrow is told the truth about the bytes. The UTF-8 and US-ASCII leaves ride
Arrow's text layouts - ASCII bytes are UTF-8 - and the windows-1252 leaves the
matching *binary* layout, because an Arrow reader told otherwise reads
mojibake and calls it text; a fixed leaf rides `FixedSizeBinary` in every
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

A document over a storage it does not describe is a foreign field wearing our
name, and it imports as its storage. A code rides its own extension name
([Codes](../codes/index.md)), and a byte column its own document
([Bytes](bytes.md)).

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
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import Field

    # Plain UTF-8 is Arrow's own datatype and crosses bare.
    assert yggdryl.utf8("text").into_arrow().metadata is None

    # US-ASCII is UTF-8, so it rides the text layout; the leaf rides the document.
    note = yggdryl.ascii("note", nullable=False)
    arrow = note.into_arrow()
    assert arrow.type == pa.string()
    assert arrow.metadata == {
        b"ARROW:extension:name": b"yggdryl.string",
        b"ARROW:extension:metadata": b'{"layout":"ascii","charset":"us-ascii"}',
    }
    assert Field.from_arrow(arrow) == note

    # A fixed width is Arrow's fixed binary, whatever the charset.
    ccy = yggdryl.fixed_ascii("ccy", 4, nullable=False)
    arrow = ccy.into_arrow()
    assert arrow.type == pa.binary(4)
    assert arrow.metadata[b"ARROW:extension:metadata"] == (
        b'{"layout":"fixed_ascii","charset":"us-ascii","fixed":4}'
    )
    assert Field.from_arrow(arrow) == ccy
    assert Field.from_arrow(pa.field("ccy", pa.binary(4))) == Field("ccy", "fixed_binary(4)")

    # A windows-1252 leaf rides binary storage, because its bytes are not UTF-8.
    latin = yggdryl.sized_cp1252("name", 32)
    arrow = latin.into_arrow()
    assert arrow.type == pa.binary()
    assert arrow.metadata[b"ARROW:extension:metadata"] == (
        b'{"layout":"sized_cp1252","charset":"windows-1252","max":32}'
    )
    assert Field.from_arrow(arrow) == latin
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
    ```

## Charsets and bounds

A bound counts **stored bytes**, not scalars: that is what the buffer holds and
what Arrow's offsets measure, and [`Charset::encoded_len`](../../charset/index.md)
counts it without building them. UTF-8 and US-ASCII are validated repertoires,
so bytes that are not what they claim are refused naming the charset, and a
US-ASCII value holds no NUL and no byte above `0x7F`. A windows-1252 leaf is a
declaration that the column holds legacy bytes, and those are *transcribed*: an
unassigned byte reads as its ISO 8859-1 scalar, which is what the WHATWG
Encoding Standard's own index maps it to.

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
refuses them naming the scalar. The value door counts rather than judges.

## Casts

A string target on any leaf but the three Arrow's own - a maximum, a fixed
width, a charset other than UTF-8 - validates every cell on the way in
(`StringIngest`); `utf8`, `large_utf8` and `utf8_view` stay Arrow's own kernel.
A source with a `yggdryl.string` document is read under its own leaf and
restated under the target's; a code source is read as its trimmed text; bare
text storage is read as text; bare binary storage is read as bytes already in
the target charset (a fixed source trimmed of NUL first). Under `safe` a
failing cell becomes null, under strict an error names the row and the column.
A fixed leaf pads on the way in, and the stored column read back under `utf8`
trims.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, StringArray};
    use yggdryl::FieldValue as _;
    use yggdryl::{ArrowCastOptions, DataType, Field, StructType};

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
    let text = DataType::from(StructType::from_fields([DataType::utf8().required_field("ccy")])?).required_field("row");
    let trimmed = text.cast_arrow_batch(batch, strict)?;
    let trimmed = trimmed.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(trimmed.value(1), "EU");

    // Under `safe` a failing cell is null; strict names the row and the column.
    let long: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EURO!"]));
    let nulled = ccy.cast_arrow_array(Arc::clone(&long), ArrowCastOptions::new())?;
    assert!(nulled.is_null(1));
    let refused = ccy.cast_arrow_array(long, strict).unwrap_err().to_string();
    assert!(refused.contains("row 1") && refused.contains("at most 4 bytes of us-ascii, got 5"), "{refused}");
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    import yggdryl

    from yggdryl import DataType

    ccy = yggdryl.fixed_ascii("ccy", 4)

    # A cast into the width pads.
    padded = ccy.cast_arrow_array(pa.array(["USD", "EU"]))
    assert padded.to_pylist() == [b"USD\x00", b"EU\x00\x00"]

    # A stored column carrying the document reads back under `utf8` trimmed.
    stored = pa.record_batch([padded], schema=pa.schema([ccy.into_arrow()]))
    text = DataType.from_fields([yggdryl.utf8("ccy")])
    assert text.cast_arrow_batch(stored).column(0).to_pylist() == ["USD", "EU"]

    # Under `safe` a failing cell is null; strict names the row and the column.
    assert ccy.cast_arrow_array(pa.array(["USD", "EURO!"])).to_pylist() == [b"USD\x00", None]
    with pytest.raises(ValueError, match="row 1: expected at most 4 bytes of us-ascii, got 5"):
        ccy.cast_arrow_array(pa.array(["USD", "EURO!"]), safe=False)
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
    ```

## Serialized shape

One `string` tag for every string, with `layout` naming the leaf (omitted when
`utf8`) and `fixed` or `max` beside it (omitted when the leaf carries no
number). The leaf names its charset, so no `charset` key is written; a document
carrying one restates the leaf in that charset's family, so
`{"type":"string","layout":"large_string","charset":"windows-1252"}` reads as
`large_cp1252`. A scalar crosses as `{"type":"string","value":...}`; the value
is bare text under the default leaf and an object naming the leaf otherwise.

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
    const field = new Field('name', 'fixed_cp1252(8)', true)
    assert.ok(Field.fromJSONBytes(field.toJSONBytes()).equals(field))
    assert.throws(() => DataType.fromJSON({ type: 'utf8' }), /unknown variant `utf8`/)
    ```

## The vocabulary a string column declares

`StringEnum` is the `FIELD:enum` dictionary: one US-ASCII value per member
name, stored on the field under that reserved key ([Protocol](../protocol.md)),
so the width stays the field's datatype and the enum crosses Arrow, a file and
another runtime intact. It is accepted on a fixed US-ASCII leaf of at most
sixteen bytes or on a [registered code](../codes/index.md), and refused by name
elsewhere, because a member's integer is `ascii_packed` - the value's bytes
padded to the width and read big-endian. The ISO listings ship with the
package, so a column declares the vocabulary it draws from without a copy per
language.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StringEnum};

    let ascii4 = DataType::fixed_ascii(4)?;
    assert_eq!(ascii4.ascii_packed(b"USD")?, 0x5553_4400);
    assert_eq!(ascii4.ascii_value(0x5553_4400)?, "USD");
    // A variable string has no width to pack into, and neither has UTF-8.
    assert!(DataType::ascii().ascii_packed(b"US").is_err());
    assert!(DataType::fixed_utf8(4)?.ascii_packed(b"US").is_err());

    // A field declares the enum its values name, as one metadata document, so
    // the enum crosses Arrow and comes back the enum that was written.
    let side = StringEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")])?;
    let field = Field::new("side", ascii4.clone(), false).try_with_string_enum(&side)?;
    assert_eq!(side.into_members(&ascii4)?[0], ("BUY".into(), 0x4200_0000));
    assert_eq!(Field::from_arrow_field(&field.into_arrow_field()?)?.string_enum()?, Some(side.clone()));

    // The ISO listings ship with the package.
    let currencies = StringEnum::from_logical_name("currency")?;
    assert_eq!(currencies.get("USD"), Some("USD"));
    // A string that does not pack is refused by name.
    let refused = Field::new("side", DataType::utf8(), false).try_with_string_enum(&side).unwrap_err().to_string();
    assert!(refused.contains("at most 16 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Field, StringEnum

    ascii4 = DataType.fixed_ascii(4)
    assert ascii4.ascii_packed("USD") == 0x55534400
    assert ascii4.ascii_value(0x55534400) == "USD"
    with pytest.raises(ValueError, match="at most 16 bytes"):
        DataType("ascii").ascii_packed("US")

    # A field declares the enum its values name, as one metadata document.
    side = StringEnum("Side", {"BUY": "B", "SELL": "S"})
    field = Field("side", ascii4, nullable=False)
    field.set_string_enum(side)
    assert side.into_members(ascii4)[0] == ("BUY", 0x42000000)
    assert Field.from_arrow(field.into_arrow()).string_enum == side

    # The ISO listings ship with the package.
    assert StringEnum.from_logical_name("currency").get("USD") == "USD"
    # A string that does not pack is refused by name.
    with pytest.raises(ValueError, match="at most 16 bytes"):
        Field("side", "utf8").set_string_enum(side)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, StringEnum } = require('yggdryl')

    const ascii4 = DataType.fixedAscii(4)
    assert.equal(ascii4.asciiPacked('USD'), 0x55534400n)
    assert.equal(ascii4.asciiValue(0x55534400n), 'USD')
    assert.throws(() => new DataType('ascii').asciiPacked('US'), /at most 16 bytes/)

    // The ISO listings ship with the package.
    const currencies = StringEnum.fromLogicalName('currency')
    assert.equal(currencies.get('USD'), 'USD')
    ```

The packing itself, the twelve registered codes and the generated enums each
language builds are on [Codes](../codes/index.md).

## Regex captures

`DataType::from_regex` builds one Struct from a byte regex's named captures, in
capture order, so [plain-text records](../../media/text/index.md) publish a
schema before a source is opened. A capture the pattern does not constrain
stays `utf8`.

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
| Rows read | none, so [plain-text records](../../media/text/index.md) publish a schema before opening a source |

## Edges

- `fixed_utf8`, `sized_ascii` with no number -> refused; the number is what makes the leaf. A bound of `0` -> refused, `at least one byte, got 0`.
- `utf8(windows-1252)`, `ascii(windows-1252)` -> refused; a charset-named spelling declares its charset in the name. `string(iso-8859-1)` -> refused; only UTF-8, US-ASCII and windows-1252 have a leaf.
- `utf8(32)`, `ascii(4)`, `cp1252(32)` -> the sized leaf written short; `large_utf8(64)`, `utf8_view(8)` -> refused. `varchar(255)` -> `sized_utf8(255)`; `char(8)` -> `fixed_utf8(8)`; bare `char` -> `utf8`.
- A bound counts stored bytes, so `sized_utf8(4)` refuses `Grüß` and `sized_cp1252(4)` holds it.
- Every UTF-8 and US-ASCII leaf reads bytes strictly, naming the charset; every windows-1252 leaf transcribes, an unassigned byte as its ISO 8859-1 scalar.
- A US-ASCII value -> no NUL, no byte above `0x7F`, refused naming the byte and its position; a variable US-ASCII value keeps its length, only a fixed leaf trims trailing NUL.
- A fixed string stores its value padded with trailing NUL and reads back trimmed.
- Text windows-1252 has no bytes for -> held as a value, refused when the column is written, naming the scalar.
- A value never carries a maximum: `Scalar::dtype()` of a cell read out of `sized_utf8(32)` is `utf8`.
- `Scalar::from("USD")` and a value read out of an `ascii` column are one value; `Str` equality, order and hash read the characters alone.
- `string_parameters` on a [code](../codes/index.md) -> `None`; a code answers `code_width`, the maximum its standard fixes over the text it stores.
- A `yggdryl.string` document over a storage it does not describe -> imports as the storage.
- A stored column carrying `yggdryl.msgdirection` or `yggdryl.direction` -> imports as the `fixed_size_binary(4)` it is: the datatype was retired, and which way a message moved is FIX's tag 385, text over its code set.
- Arrow JS rows carry no extension identity, so a `fixed_ascii(n)` column arrives as its padded bytes through `readRecords`; declare `utf8` to read text.
- A `StringIngest` refusal under `safe` -> null, which a required column then fills with the default; under strict -> `field "<name>" row <n>: expected ..., got ...`.
- Avro and Iceberg -> a UTF-8 or US-ASCII leaf crosses as `string`, a fixed one trimmed of padding; a windows-1252 leaf is refused by name.
- Merging follows [Field](../field.md): two strings meet parameter by parameter, and a string never meets a byte column.
- `from_regex(pattern, false)` -> every capture stays `utf8`; invalid regex syntax or an expression past the recursion limit -> datatype error.
- `\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}` -> a millisecond datetime and `\d{2}:\d{2}:\d{2},\d{3}` -> a millisecond time: the comma is a decimal sign inside a clock. Outside one it is not, so `\d+,\d+` and `\d{1,3}(?:,\d{3})*` stay `utf8` and a bare fraction such as `,\d{3}` carries no clock to be part of.
- A capture admitting several widths takes the widest: `\.\d{1,5}` -> microseconds, and an optional or variable fraction publishes the widest unit it admits even where every row spells none, so `\d{2}:\d{2}:\d{2}(?:\.\d{2})?` -> `time32(ms)`. A capture spelling one width it once had no candidate for - two, four, seven or eight digits - is now that width's datetime rather than `utf8`, and a row the reader refuses in such a column is null under `safe` rather than the text it used to stay.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- ascii::fields ascii::leaves cast::typed::strings regex::captures regex::fractions string::enumerated string::leaves string::widths
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- string::codes
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^string/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "string or ascii"
    python/.venv/bin/python -m pytest python/tests/text/test_init.py -k regex
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="string|ASCII|ascii|regex captures" node/tests/datatype.test.js node/tests/fields.test.js
    npm run --prefix node bench:types
    ```
