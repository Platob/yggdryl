# Codes

The twelve registered codes: an identity over a published registry, the width its standard fixes, and the Arrow extension name that identity rides.

A code is not a string with a charset - a currency is ISO 4217 the way a [URL](../../uri/url-urn.md) is RFC 3986. It stores as the US-ASCII text it is, Arrow's `Utf8` under the code's own extension name, held to the width its standard fixes. It is its own datatype, kind `code`, answers `is_code`, `code_name` and `code_width`, and never `string_parameters`. The width is a maximum rather than a layout, so `fixed_byte_width` answers `None`. Text of any length in that repertoire is the [`ascii` string](../text/string.md).

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | Twelve `DataType` variants, twelve `Field` leaves, twelve `Scalar` variants, and the `Code` family value over them |
| Validates | At the value door, once: US-ASCII, no NUL, at most the code's width, then the code's own rule - a check digit, a category grid, a published spelling |
| Lazy | Nothing - a code has no children, no registry lookup and no deferred parse |
| Cached | The Arrow projection of a [`Field`](../field.md), built once per field |
| Refuses | A byte past `0x7F`, a NUL, text longer than the width, and whatever the code's own rule refuses; `string_parameters`, which a code has none of |
| Errors | Rust `Error::InvalidDataType { kind, reason }` where `kind` is the code's own name; Python `ValueError`; JavaScript throws |
| Storage | The text itself: nothing padded, nothing to trim, so a column dictionary-encodes and carries string statistics like any other text |
| Identity | The extension *name*, never the storage: `yggdryl.currency` over `utf8` is a currency, and the same `utf8` under `yggdryl.string` or under no name at all is the text it is |
| Value rank | The twelve share one value rank, so what separates two codes of the same bytes is the identity their datatypes sort by: `Side("BUY")` and `TimeInForce("BUY")` are two values |
| Rust only | `DataType::CODES`, the twelve leaf value types, the `Code` family enum, `CodeValue` and its `merge_with`, `Scalar::code_storage` and `Scalar::is_code` |

The contract every registered code answers lives in `rust/src/code.rs`: the `CodeValue` trait - `WIDTH`, `as_str`, `storage`, `merge_with` - and the two crate-internal builders `code_leaf!` and `code_value!` that a code file declares its value with. Each of the twelve is then one file of its own, holding its datatype, its field marker and its value in that order.

## Pages

| Page | Registry | Most bytes | Arrow extension |
| --- | --- | ---: | --- |
| [Currency](currency.md) | ISO 4217 | 3 | `yggdryl.currency` |
| [Country](country.md) | ISO 3166-1 alpha-2 | 2 | `yggdryl.country` |
| [MIC](mic.md) | ISO 10383 market identifier | 4 | `yggdryl.mic` |
| [CFI](cfi.md) | ISO 10962 classification | 6 | `yggdryl.cfi` |
| [ISIN](isin.md) | ISO 6166, closed by a check digit | 12 | `yggdryl.isin` |
| [CUSIP](cusip.md) | CUSIP Global Services, closed by a check digit | 9 | `yggdryl.cusip` |
| [SEDOL](sedol.md) | London Stock Exchange, closed by a check digit | 7 | `yggdryl.sedol` |
| [Bloomberg](bloomberg.md) | A terminal identifier no standard closes | 32 | `yggdryl.bloomberg` |
| [FIGI](figi.md) | ANSI X9.145, closed by a check digit | 12 | `yggdryl.figi` |
| [Side](side.md) | FIX `Side(54)`, read by spelling | 8 | `yggdryl.side` |
| [State](state.md) | A ranked lifecycle over FIX and a scheduler | 10 | `yggdryl.state` |
| [TimeInForce](timeinforce.md) | FIX `TimeInForce(59)`, the wire value | 8 | `yggdryl.timeinforce` |

## What every code answers

`DataType::CODES` is the one listing - the parser, the Arrow extension table and every binding read the codes from it; `is_code` and `code_name` tell one from the text beside it. The [playground](../playground.md) renders sample codes, refusals and vocabularies as the package answered them.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind, Scalar};

    // A registered code is a datatype, not a name over a width.
    let currency = DataType::currency();
    assert_eq!(DataType::from_str("currency")?, currency);
    assert_eq!(currency.to_string(), "currency");
    assert_eq!(currency.kind(), DataTypeKind::Code);
    assert!(currency.is_code());
    assert_eq!(currency.code_name(), Some("currency"));
    // The width bounds a value; a code stores as its text, so it names no
    // fixed layout.
    assert_eq!(currency.code_width(), Some(3));
    assert_eq!(currency.fixed_byte_width(), None);
    assert!(currency.string_parameters().is_none());
    assert_ne!(currency, DataType::fixed_ascii(3)?);

    assert_eq!(
        DataType::CODES,
        &[
            ("country", DataType::Country, 2),
            ("currency", DataType::Currency, 3),
            ("mic", DataType::MicCode, 4),
            ("cfi", DataType::CfiCode, 6),
            ("isin", DataType::IsinCode, 12),
            ("cusip", DataType::CusipCode, 9),
            ("sedol", DataType::SedolCode, 7),
            ("side", DataType::Side, 8),
            ("state", DataType::State, 10),
            ("timeinforce", DataType::TimeInForce, 8),
            ("bloomberg", DataType::BloombergCode, 32),
            ("figi", DataType::FIGICode, 12),
        ]
    );

    // A value is the text, and carries its identity.
    let usd = currency.scalar("USD")?;
    assert_eq!(usd.as_str(), Some("USD"));
    assert_eq!(usd.kind(), "currency");
    assert!(usd.is_code());
    assert_ne!(DataType::Side.scalar("BUY")?, DataType::TimeInForce.scalar("BUY")?);
    // A plain string of the same bytes is a string.
    assert_eq!(Scalar::from("USD").kind(), "string");
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar

    # A registered code is a datatype, not a name over a width.
    currency = DataType("currency")
    assert str(currency) == "currency"
    assert currency.kind == "code"
    assert currency.is_code
    assert currency.code_name == "currency"
    # The width bounds a value; a code stores as its text, so it names no
    # fixed layout.
    assert currency.code_width == 3
    assert currency.fixed_byte_width is None
    assert currency.string_parameters is None
    assert currency != DataType.fixed_ascii(3)
    assert [(DataType(name).id, DataType(name).code_width) for name in
            ("country", "currency", "mic", "cfi", "isin", "cusip", "sedol",
             "side", "state", "timeinforce", "bloomberg", "figi")] == [
        ("country", 2), ("currency", 3), ("mic", 4), ("cfi", 6), ("isin", 12),
        ("cusip", 9), ("sedol", 7), ("side", 8), ("state", 10), ("timeinforce", 8),
        ("bloomberg", 32), ("figi", 12),
    ]

    # A value is the text, and carries its identity.
    usd = currency.scalar("USD")
    assert usd.as_str() == "USD"
    assert usd.kind == "currency"
    assert usd.family == "code"
    assert DataType("side").scalar("BUY") != DataType("timeinforce").scalar("BUY")
    assert Scalar.from_("USD").kind == "string"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    // A registered code is a datatype, not a name over a width.
    const currency = new DataType('currency')
    assert.equal(currency.id, 'currency')
    assert.equal(currency.toString(), 'currency')
    assert.equal(currency.kind, 'code')
    // The width bounds a value; a code stores as its text, so it names no
    // fixed layout.
    assert.equal(currency.codeWidth, 3)
    assert.equal(currency.fixedByteWidth, null)
    assert.equal(currency.stringParameters, null)
    assert.ok(!currency.equals(DataType.fixedAscii(3)))
    assert.deepEqual(
      ['country', 'currency', 'mic', 'cfi', 'isin', 'cusip', 'sedol', 'side', 'state', 'timeinforce', 'bloomberg', 'figi']
        .map((name) => new DataType(name).codeWidth),
      [2, 3, 4, 6, 12, 9, 7, 8, 10, 8, 32, 12],
    )

    // A value is the text, and carries its identity.
    const usd = currency.scalar('USD')
    assert.equal(usd.asJs(), 'USD')
    assert.equal(usd.kind, 'currency')
    assert.equal(usd.family, 'code')
    assert.equal(Scalar.from('USD').kind, 'string')
    ```

## The `Code` family value

The twelve as one value: a variant per code, named as the `Scalar` variant is, and a `FamilyValue` beside `CodeValue`. `Scalar::as_code` narrows to it and `into_scalar` widens back ([Scalar](../scalar.md#families)). Rust only - Python and JavaScript read the family off the value itself, as `family` above.

```rust
use yggdryl::{Code, Currency, DataType, DataTypeKind, FamilyValue, Scalar};

let held = Code::from(Currency::new("EUR")?);
assert_eq!(Code::KIND, DataTypeKind::Code);
assert_eq!(held.dtype()?, DataType::Currency);
assert_eq!(held.clone().into_scalar(), Scalar::Currency(Currency::new("EUR")?));
assert_eq!(Scalar::Currency(Currency::new("EUR")?).as_code(), Some(held));

// The text a code is made of is not the code.
assert_eq!(Code::from_scalar(&Scalar::from("EUR")), None);
```

`CodeValue::merge_with` is the better statement of two codes of one kind, and what a [graph element](../../graph.md) folds two statements of one fact with. What "less" means is each code's own: a `cfi` fills every `X` from the other where the two describe one instrument, a `state` that reached none takes the other and otherwise the further along stands, a `side` `UNKNOWN`, a `currency` `XXX` and a `mic` `XXXX` take the other, and an identifier stands as it is. Rust only.

```rust
use yggdryl::{CodeValue, Currency, IsinCode, MicCode, State};

// A code stated as none takes the other; anything stated stands.
assert_eq!(Currency::none().merge_with(&Currency::new("USD")?).as_str(), "USD");
assert_eq!(Currency::new("USD")?.merge_with(&Currency::new("EUR")?).as_str(), "USD");
assert_eq!(MicCode::none().merge_with(&MicCode::new("XPAR")?).as_str(), "XPAR");

// A lifecycle takes the state further along, whichever side it is on.
assert_eq!(State::read("New")?.merge_with(&State::read("Filled")?), State::read("Filled")?);

// An identifier has nothing partial about it: this one stands.
let apple = IsinCode::new("US0378331005")?;
assert_eq!(apple.clone().merge_with(&IsinCode::new("US5949181045")?), apple);
```

## Arrow storage

Every code rides Arrow's `Utf8` - which is what the text is - and the `yggdryl.<code>` name beside it carries the identity, so the trip out and back keeps the code rather than anonymous bytes. The same storage under no name at all is plain text.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let venue = Field::new("venue", DataType::MicCode, false);
    let arrow = venue.clone().into_arrow_field()?;
    // The storage is the text; the name beside it is the identity.
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.mic");
    assert_eq!(Field::from_arrow_field(&arrow)?, venue);

    // The same storage under no name at all is plain text.
    let bare = arrow_schema::Field::new("venue", ArrowDataType::Utf8, false);
    assert_eq!(Field::from_arrow_field(&bare)?.dtype(), &DataType::utf8());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import Field

    venue = yggdryl.mic("venue", nullable=False)
    venue_arrow = venue.into_arrow()
    # The storage is the text; the name beside it is the identity.
    assert venue_arrow.type == pa.string()
    assert venue_arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.mic"
    assert Field.from_arrow(venue_arrow) == venue
    # A value stores as itself, whatever the width leaves unused.
    assert venue.arrow_scalar("XPA") == pa.scalar("XPA", pa.string())
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const venue = fields.struct('row', [fields.mic('venue', { nullable: false })], {
      nullable: false,
    })
    const stored = Serie.fromArrowBatch(
      new arrow.Table({ venue: arrow.vectorFromArray(['XPA'], new arrow.Utf8()) }),
      venue,
    ).intoArrowBatch()
    const venueArrow = stored.schema.fields[0]
    // The storage is the text; the name beside it is the identity.
    assert.equal(String(venueArrow.type), 'Utf8')
    assert.equal(venueArrow.metadata.get('ARROW:extension:name'), 'yggdryl.mic')
    // A value stores as itself, whatever the width leaves unused.
    assert.deepEqual([...stored.getChild('venue')], ['XPA'])
    const text = fields.struct('row', [fields.utf8('venue', { nullable: false })], {
      nullable: false,
    })
    const plain = Serie.fromArrowBatch(stored, text).intoArrowBatch()
    assert.deepEqual([...plain.getChild('venue')], ['XPA'])
    // The same storage under no name at all is plain text.
    assert.equal(plain.schema.fields[0].metadata.get('ARROW:extension:name'), undefined)
    ```

## Packed integers and the declared vocabulary

`ascii_packed` is the value's bytes padded to the width and read big-endian: one integer everywhere, ordered as the text, never negative. The padding belongs to the packing - a code's column stores the text alone - which is what keeps a `FIELD:enum` document the same integers whatever a column stores its values as. It answers for a fixed US-ASCII string of at most sixteen bytes or a code, and refuses everything else by name. `StringEnum` names those integers: one US-ASCII value per member, stored on the field under the reserved key `FIELD:enum` ([Protocol](../protocol.md)), so the width stays the field's datatype and the enum crosses Arrow, a file, and another runtime intact.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StringEnum};

    // A value's integer is its own storage bytes read big-endian, so it is the
    // same integer in every process and orders exactly as the text does.
    let ascii4 = DataType::fixed_ascii(4)?;
    assert_eq!(ascii4.ascii_packed(b"USD")?, 0x5553_4400);
    assert_eq!(ascii4.ascii_packed(b"USD\0")?, 0x5553_4400);
    assert_eq!(ascii4.ascii_value(0x5553_4400)?, "USD");
    assert_eq!(DataType::Currency.ascii_packed(b"USD")?, 0x0055_5344);
    // Sixteen bytes fill the whole `i128`; a wider width has no packed code.
    assert_eq!(
        DataType::fixed_ascii(16)?.ascii_packed(b"US0378331005")?,
        0x5553_3033_3738_3333_3130_3035_0000_0000
    );
    assert!(DataType::fixed_ascii(17)?.ascii_packed(b"US").is_err());
    // A variable string has no width to pack into, and neither has UTF-8.
    assert!(DataType::ascii().ascii_packed(b"US").is_err());
    assert!(DataType::fixed_utf8(4)?.ascii_packed(b"US").is_err());

    // An enum is that naming as a value: one US-ASCII value per member name.
    let venues = StringEnum::from_members("Venue", [("XNAS", "XNAS"), ("N_A", "n/a")])?;
    assert_eq!(venues.get("N_A"), Some("n/a"));
    assert_eq!(
        venues.into_members(&DataType::MicCode)?,
        [("N_A".into(), 0x6E2F_6100), ("XNAS".into(), 0x584E_4153)]
    );

    // The same rule names one value at a time, for a vocabulary declared
    // member by member rather than generated from a whole listing.
    assert_eq!(StringEnum::member_name("n/a").as_str(), "N_A");

    // The ISO listings ship with the package, so a code column declares the
    // vocabulary it draws from without a copy per language.
    let currencies = StringEnum::from_logical_name("currency")?;
    assert_eq!(currencies.len(), StringEnum::CURRENCIES.len());
    assert_eq!(currencies.get("USD"), Some("USD"));
    assert_eq!(StringEnum::from_logical_name("Exchange")?.len(), StringEnum::MICS.len());
    // A registered name with no listing answers an enum of no members.
    assert!(StringEnum::from_logical_name("tenor")?.is_empty());

    // A field declares the enum its values name, as one metadata document, so
    // the enum crosses Arrow and comes back the enum that was written.
    let side = StringEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")])?;
    let field = Field::new("side", ascii4.clone(), false).try_with_string_enum(&side)?;
    assert_eq!(side.into_members(&ascii4)?[0], ("BUY".into(), 0x4200_0000));
    assert_eq!(Field::from_arrow_field(&field.into_arrow_field()?)?.string_enum()?, Some(side.clone()));
    // A string that does not pack is refused by name.
    let refused = Field::new("side", DataType::utf8(), false).try_with_string_enum(&side).unwrap_err().to_string();
    assert!(refused.contains("at most 16 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import enum

    import pytest

    from yggdryl import DataType, Field, StringEnum

    # A value's integer is its own storage bytes read big-endian, so it is the
    # same integer in every process and orders exactly as the text does.
    ascii4 = DataType.fixed_ascii(4)
    assert ascii4.ascii_packed("USD") == 0x55534400
    assert ascii4.ascii_packed("USD\x00") == 0x55534400
    assert ascii4.ascii_value(0x55534400) == "USD"
    assert DataType("currency").ascii_packed("USD") == 0x555344
    # Sixteen bytes fill the whole 128-bit integer, which Python holds natively.
    assert DataType.fixed_ascii(16).ascii_packed("US0378331005") == (
        0x55533033373833333130303500000000
    )
    # A variable string has no width to pack into, and neither has UTF-8.
    with pytest.raises(ValueError, match="at most 16 bytes"):
        DataType("ascii").ascii_packed("US")
    with pytest.raises(ValueError, match="at most 16 bytes"):
        DataType.fixed_utf8(4).ascii_packed("US")

    # An enum is that naming as a value: one US-ASCII value per member name.
    venues = StringEnum("Venue", {"XNAS": "XNAS", "N_A": "n/a"})
    assert venues.get("N_A") == "n/a"
    assert venues.into_members("mic") == [("N_A", 0x6E2F6100), ("XNAS", 0x584E4153)]

    # ... and as a Python `IntEnum`, keyed by the same integers.
    Venue = venues.into_intenum("mic")
    assert issubclass(Venue, enum.IntEnum)
    assert Venue(0x584E4153).name == "XNAS"

    # The same rule names one value at a time, for a vocabulary declared
    # member by member rather than generated from a whole listing.
    assert StringEnum.member_name("n/a") == "N_A"

    # The ISO listings ship with the package, so a code column declares the
    # vocabulary it draws from without a copy per language.
    currencies = StringEnum.from_logical_name("currency")
    assert len(currencies) == len(StringEnum.prebuilt()["currency"])
    assert currencies.get("USD") == "USD"
    assert len(StringEnum.from_logical_name("Exchange")) == len(StringEnum.prebuilt()["mic"])
    # A registered name with no listing answers an enum of no members.
    assert len(StringEnum.from_logical_name("tenor")) == 0

    # A field declares the enum its values name, as one metadata document, so
    # the enum crosses Arrow and comes back the enum that was written.
    side = StringEnum("Side", {"BUY": "B", "SELL": "S"})
    field = Field("side", ascii4, nullable=False)
    field.set_string_enum(side)
    assert side.into_members(ascii4)[0] == ("BUY", 0x42000000)
    assert Field.from_arrow(field.into_arrow()).string_enum == side
    # A string that does not pack is refused by name.
    with pytest.raises(ValueError, match="at most 16 bytes"):
        Field("side", "utf8").set_string_enum(side)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, StringEnum } = require('yggdryl')

    // A value's integer is its own storage bytes read big-endian, so it is the
    // same integer in every process and orders exactly as the text does.
    const ascii4 = DataType.fixedAscii(4)
    assert.equal(ascii4.asciiPacked('USD'), 0x55534400n)
    assert.equal(ascii4.asciiPacked('USD\0'), 0x55534400n)
    assert.equal(ascii4.asciiValue(0x55534400n), 'USD')
    assert.equal(new DataType('currency').asciiPacked('USD'), 0x555344n)
    // Sixteen bytes fill the whole 128-bit integer, so every code is a bigint.
    assert.equal(
      DataType.fixedAscii(16).asciiPacked('US0378331005'),
      0x55533033373833333130303500000000n,
    )
    // A variable string has no width to pack into, and neither has UTF-8.
    assert.throws(() => new DataType('ascii').asciiPacked('US'), /at most 16 bytes/)
    assert.throws(() => DataType.fixedUtf8(4).asciiPacked('US'), /at most 16 bytes/)

    // An enum is that naming as a value: one US-ASCII value per member name.
    const venues = new StringEnum('Venue', { XNAS: 'XNAS', N_A: 'n/a' })
    assert.equal(venues.get('N_A'), 'n/a')
    assert.deepEqual(venues.intoMembers('mic'), { XNAS: 0x584e4153n, N_A: 0x6e2f6100n })

    // ... and as the generated enum: a frozen name-to-code object, tagged with
    // the enum's own name.
    const Venue = venues.intoEnum('mic')
    assert.equal(Venue.XNAS, new DataType('mic').asciiPacked('XNAS'))
    assert.equal(Object.prototype.toString.call(Venue), '[object Venue]')

    // The same rule names one value at a time, for a vocabulary declared
    // member by member rather than generated from a whole listing.
    assert.equal(StringEnum.memberName('n/a'), 'N_A')

    // The ISO listings ship with the package, so a code column declares the
    // vocabulary it draws from without a copy per language.
    const currencies = StringEnum.fromLogicalName('currency')
    assert.equal(currencies.length, StringEnum.prebuilt().currency.length)
    assert.equal(currencies.get('USD'), 'USD')
    assert.equal(StringEnum.fromLogicalName('tenor').length, 0)

    // A field declares the enum its values name, as one metadata document, so
    // every serialization carries it and it comes back the enum that wrote it.
    const side = new StringEnum('Side', { BUY: 'B', SELL: 'S' })
    const field = new Field('side', ascii4, false)
    field.setStringEnum(side)
    assert.deepEqual(side.intoMembers(ascii4), { BUY: 0x42000000n, SELL: 0x53000000n })
    assert.ok(Field.fromJSONBytes(field.toJSONBytes()).stringEnum.equals(side))
    // A string that does not pack is refused by name.
    assert.throws(() => new Field('side', 'utf8').setStringEnum(side), /at most 16 bytes/)
    ```

| Byte | Member name |
| --- | --- |
| letter | uppercased |
| digit | kept |
| other | `_` |
| leading digit | `_` prefixed |
| opens and closes with `_` | trailing `_` dropped |

Declaring a vocabulary *over* one of these widths is Python-only: `yggdryl.enums`
builds an `IntEnum` base whose members are their own storage bytes read
big-endian, so a member is the text and the integer at once. Rust and JavaScript
express the same column as the datatype alone. The four registered bases -
`Currency`, `Country`, `MicCode`, `CFI` - ship declared; `fixed_ascii(width)` builds
one over any fixed width.

=== "Python"

    ```python
    from yggdryl.enums import Currency, fixed_ascii

    # A member is its value's own storage bytes, read big-endian: three ASCII
    # letters of a currency are three bytes of an integer.
    assert int(Currency.USD) == 0x555344
    assert str(Currency.USD) == "USD"

    class Venue(fixed_ascii(4)):
        XNAS = "XNAS"

    assert int(Venue.XNAS) == 0x584E4153

    # The vocabulary is open: a value nothing declared reads back as a member
    # under its own code, and every spelling of it is that one member.
    assert Venue("XLON") is Venue("XLON")
    assert str(Venue("XLON")) == "XLON"

    # A value the width refuses is an error, not a silent unknown member.
    try:
        Venue("TOOLONG")
    except ValueError as refusal:
        assert "at most 4 bytes" in str(refusal)
    else:
        raise AssertionError("a value wider than the declaration was accepted")
    ```

## FIX message definitions

FIX tag 35 stores complete `utf8` text, including codes such as `P Report Ack`.
The [FIX registry](../../fix/registry.md)
owns `MsgType`: the registry's immutable message Struct definition, a component
carrying `FIX:msgtype` and optional fixed `FIX:msgcat`, obtained through registry
lookup. Its wire code stays intact; message definitions have no generic datatype
or code field helper. A fixed row carries `msgcat` at crate tag 65054 and its
six normalized identifier columns: `isincode(65055)`, `cusipcode(65057)`,
`sedolcode(65058)`, `bloombergcode(65059)`, `miccode(65060)` and
`figicode(65061)`. `CFICode(461)` is the standard classification field, so no
crate 65056 exists. `SecurityIDSource(22)=S` and
`SecurityAltIDSource(456)=S` lift a valid FIGI; source `A` remains Bloomberg.

## Edges

- A byte past `0x7F`, a NUL, or a value longer than the width -> refused naming the width (`at most 4 bytes`), and the row in a cast.
- Stored under a code -> the text itself, so nothing is padded and nothing has to be trimmed back. A cast from a fixed-width column still trims the NUL that column's slot wrote; the padding was the slot's, never the value's. Text carrying trailing NULs canonicalizes to the trimmed value.
- `Scalar::kind()` -> the code's id: `currency`, `side`, `state`; a plain `utf8` string's kind is `string`, and any other leaf's is its name, `fixed_utf8` or `cp1252`.
- A code's equality, order and hash carry the identity first, then the text: `Side("1") != TimeInForce("1")`. A code and a plain string of the same bytes are two values.
- `utf8` under `yggdryl.currency` -> `currency`; under `yggdryl.string` with a document -> the string it describes; under no name -> `utf8`. The extension *name* is what separates them, so `yggdryl.currency` over any other storage imports as that storage.
- Default value: a code defaults to the empty text its storage does, answered as the code's own scalar, and an empty text cell entering the column reads as that member ([Cast](../cast.md#empty-text)). [ISIN](isin.md), [CUSIP](cusip.md), [SEDOL](sedol.md), [FIGI](figi.md), [Bloomberg](bloomberg.md), [Side](side.md) and [State](state.md) are the exceptions, because their value door gates the space rather than holding it, so none has a neutral member: `default_value` refuses naming the code rather than answering a value no registry issued, and an empty text cell entering one of them is null, as it is for a UUID.
- A cast refusal under `safe` -> null, which a required column fills with the default; under strict -> the row and the column, for a code exactly as for a string ([Cast](../cast.md)).
- [Merged](../field.md#merging-two-schemas) widening: a code beside itself -> kept; beside `fixed_ascii(n)`, `ascii` or `utf8` -> that string. Narrowing (`upscale=false`): a code beside any plainer shape storing it -> the code; beside narrower text -> that text.
- `currency` beside `country` -> `sized_ascii(3)` widening and `sized_ascii(2)` narrowing, the bounded text both fit, never one code holding the other's values.
- A code shares no fixed width with anything, because its own width bounds variable text: beside `fixed_size_binary(n)` -> `binary` in either direction.
- Iceberg, Spark, Polars, pandas, Avro, filter literals -> text, [rewritten](../datatype.md) to `string`/`utf8`. Every registered code, not a subset: the listing each of these paths reads is `DataType::CODES`, through `DataType::is_code` and `DataType::code_width`, so a code cannot be spellable in one and unspellable in the next.
- Parquet -> the `String` logical type and the byte-array bounds that come with it, so a planner reads statistics over the codes themselves and a reader outside this crate gets a column it can already use.
- An Arrow cast into any code -> one text column, from a binary source of any framing or from anything Arrow renders as text; a text column already holding what the code promises is shared rather than copied; anything else -> refused naming the code and the source.
- An Arrow cast out of a code -> every string leaf and every byte framing, each reading the text the column holds; a byte width the text does not fill -> refused naming both sides and the row.
- `ascii_packed` on a variable string, on UTF-8, or on a width past 16 bytes -> refused, `at most 16 bytes`; otherwise an `i32`, an `i64`, or a whole `i128` by width, and the integer a stable hash hashes.
- A `StringEnum` on a string that is not fixed US-ASCII of at most sixteen bytes -> refused by name at `set_string_enum` and `into_members`; error kind `string-enum`. The `FIELD:enum` document never carries the width, so one enum is one canonical text.
- `from_logical_name` -> the shipped `COUNTRIES`, `CURRENCIES`, `MICS`, `SIDES`, `DIRECTIONS`, `STATES`, `TIMESINFORCE` listings, `prebuilt()` in either binding; `"Exchange"` -> `MICS`; a registered name with no listing, such as `tenor` or any of the four checked identifiers, -> an empty enum.
- JavaScript `readRecords` -> Arrow JS rows carry no extension identity, so a code column arrives as the text it stores, under no identity.
- A dictionary-encoded code keeps its identity: Arrow's dictionary holds a bare datatype for its values, so the field is where the extension name rides.
- Digests: a code value feeds its own id as the tag, so a stored digest of a code cell differs from the same bytes under `fixed_ascii(n)`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- ascii::fields ascii::leaves cfi_code::coded code::datatypes code::securities cusip_code::securities figi_code::securities sedol_code::securities state::coded string::enumerated string::listings timeinforce::coded
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- string::codes
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^ascii/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "registered_code or prebuilt_vocabulary or enum_member or code_datatype"
    python/.venv/bin/python -m pytest python/tests/enums/test_init.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code|vocabulary|enum|packs" node/tests/datatype.test.js node/tests/fields.test.js
    npm run --prefix node bench:types
    ```
