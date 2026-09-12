# Codes

The nine registered codes, the packed integer a fixed US-ASCII string or a code reads as, and the `StringEnum` vocabulary a field declares.

A code is an identity with a storage, not a string with a charset: a currency is three US-ASCII bytes the way a [UUID](uuid.md) is sixteen binary ones. It is its own datatype, kind `code`, answers `is_code`, `code_name` and `fixed_byte_width`, and never `string_parameters`. Text of any length in that repertoire is the [`ascii` string](text.md).

## Contract

| Spelling | Width | Arrow storage, extension |
| --- | ---: | --- |
| `country`, ISO 3166-1 alpha-2 | 2 | `fixed_size_binary(2)`, `yggdryl.country` |
| `currency`, ISO 4217 | 3 | `fixed_size_binary(3)`, `yggdryl.currency` |
| `mic`, ISO 10383 | 4 | `fixed_size_binary(4)`, `yggdryl.mic` |
| `cfi`, ISO 10962 | 6 | `fixed_size_binary(6)`, `yggdryl.cfi` |
| `isin`, ISO 6166 | 12 | `fixed_size_binary(12)`, `yggdryl.isin` |
| `side`, FIX `Side(54)` | 4 | `fixed_size_binary(4)`, `yggdryl.side` |
| `msgdirection`, which way a captured line moved | 4 | `fixed_size_binary(4)`, `yggdryl.msgdirection` |
| `state`, a ranked lifecycle | 10 | `fixed_size_binary(10)`, `yggdryl.state` |
| `timeinforce`, FIX `TimeInForce(59)` | 8 | `fixed_size_binary(8)`, `yggdryl.timeinforce` |

| | |
| --- | --- |
| Value | `Scalar::Code(Code)`: the trimmed text; equality, order and hash carry the identity, so `Side("1")` and `TimeInForce("1")` are two values |
| Storage | padded with trailing NUL to the width; every reading trims it |
| `ascii_packed` | the stored bytes read big-endian into one `i128`: a fixed US-ASCII string of at most sixteen bytes or a code; everything else refused |
| `StringEnum` | a name plus one US-ASCII value per member under `field:enum`; accepted on a fixed US-ASCII string of at most sixteen bytes or a code |
| Rust only | `DataType::CODES`, the `Code` enum and its leaves, `State::rank` and the lifecycle predicates |

## Use

`DataType::CODES` lists the codes; `is_code` and `code_name` tell one from a fixed string of the same width. The [playground](playground.md) renders every code, refusal and vocabulary as the package answered them.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, DataTypeKind, Field, Scalar};

    // A registered code is a datatype, not a name over a width.
    let currency = DataType::currency();
    assert_eq!(DataType::from_str("currency")?, currency);
    assert_eq!(currency.to_string(), "currency");
    assert_eq!(currency.kind(), DataTypeKind::Code);
    assert!(currency.is_code());
    assert_eq!(currency.code_name(), Some("currency"));
    assert_eq!(currency.fixed_byte_width(), Some(3));
    assert!(currency.string_parameters().is_none());
    assert_ne!(currency, DataType::fixed_ascii(3)?);
    assert_eq!(
        DataType::CODES,
        &[
            ("country", DataType::Country, 2),
            ("currency", DataType::Currency, 3),
            ("mic", DataType::Mic, 4),
            // Six bytes: `cfi` stores what it is, not the eight some other
            // width would pad it to.
            ("cfi", DataType::Cfi, 6),
            // Twelve bytes closed by a check digit, so a value is an
            // identifier or is refused, never a typo stored as a security.
            ("isin", DataType::Isin, 12),
            // The FIX-facing codes, each at the width it needs: a state
            // carries two digits of rank before its name.
            ("side", DataType::Side, 4),
            ("msgdirection", DataType::MsgDirection, 4),
            ("state", DataType::State, 10),
            ("timeinforce", DataType::TimeInForce, 8),
        ]
    );

    // A value is the trimmed text, and carries its identity.
    let usd = currency.scalar("USD")?;
    assert_eq!(usd.as_str(), Some("USD"));
    assert_eq!(usd.kind(), "currency");
    assert_ne!(DataType::Side.scalar("1")?, DataType::TimeInForce.scalar("1")?);
    // A plain string of the same width is a string.
    assert_eq!(Scalar::from("USD").kind(), "string");

    // A code rides its own Arrow extension, so the identity survives the trip.
    let venue = Field::new("venue", DataType::Mic, false);
    let arrow = venue.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(4));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.mic");
    assert_eq!(Field::from_arrow(&arrow)?, venue);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import DataType, Field, types

    # A registered code is a datatype, not a name over a width.
    currency = DataType("currency")
    assert str(currency) == "currency"
    assert currency.kind == "code"
    assert currency.is_code
    assert currency.code_name == "currency"
    assert currency.fixed_byte_width == 3
    assert currency.string_parameters is None
    assert currency != DataType.fixed_ascii(3)
    assert [(DataType(name).id, DataType(name).fixed_byte_width) for name in
            ("country", "currency", "mic", "cfi", "isin", "side", "msgdirection", "state", "timeinforce")] == [
        ("country", 2), ("currency", 3), ("mic", 4), ("cfi", 6), ("isin", 12),
        ("side", 4), ("msgdirection", 4), ("state", 10), ("timeinforce", 8),
    ]

    # A value is the trimmed text, and carries its identity.
    usd = currency.scalar("USD")
    assert usd.as_str() == "USD"
    assert usd.kind == "currency"
    assert DataType("side").scalar("1") != DataType("timeinforce").scalar("1")
    # An ISIN is closed by its own check digit, so one digit off is refused
    # and lower case folds to the number it spells.
    assert DataType("isin").scalar("us0378331005").as_py() == "US0378331005"
    with pytest.raises(ValueError, match="check digit"):
        DataType("isin").scalar("US0378331006")

    # A code rides its own Arrow extension, so the identity survives the trip.
    venue = types.mic("venue", nullable=False)
    venue_arrow = venue.into_arrow()
    assert venue_arrow.type == pa.binary(4)
    assert venue_arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.mic"
    assert Field.from_arrow(venue_arrow) == venue
    # Storage pads to the width; every reading trims the padding.
    assert venue.arrow_scalar("XPA") == pa.scalar(b"XPA\x00", pa.binary(4))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { DataType, fields } = require('yggdryl')

    // A registered code is a datatype, not a name over a width.
    const currency = new DataType('currency')
    assert.equal(currency.id, 'currency')
    assert.equal(currency.toString(), 'currency')
    assert.equal(currency.kind, 'code')
    assert.equal(currency.fixedByteWidth, 3)
    assert.equal(currency.stringParameters, null)
    assert.ok(!currency.equals(DataType.fixedAscii(3)))
    assert.deepEqual(
      ['country', 'currency', 'mic', 'cfi', 'isin', 'side', 'msgdirection', 'state', 'timeinforce']
        .map((name) => new DataType(name).fixedByteWidth),
      [2, 3, 4, 6, 12, 4, 4, 10, 8],
    )

    // A code rides its own Arrow extension, so the identity survives the trip.
    const venue = fields.struct('row', [fields.mic('venue', { nullable: false })], {
      nullable: false,
    })
    const stored = venue.castArrow(
      new arrow.Table({ venue: arrow.vectorFromArray(['XPA'], new arrow.Utf8()) }),
    )
    const venueArrow = stored.schema.fields[0]
    assert.equal(String(venueArrow.type), 'FixedSizeBinary[4]')
    assert.equal(venueArrow.metadata.get('ARROW:extension:name'), 'yggdryl.mic')
    // Storage pads to the width; a column read under `utf8` trims.
    assert.deepEqual([...stored.getChild('venue').get(0)], [0x58, 0x50, 0x41, 0])
    const text = fields.struct('row', [fields.utf8('venue', { nullable: false })], {
      nullable: false,
    })
    assert.deepEqual([...text.castArrow(stored).getChild('venue')], ['XPA'])
    ```

## FIX message definitions

FIX tag 35 and the [capture `msgtype` column](../media/text.md#classifying-each-record)
store complete `utf8` text, including codes such as `P Report Ack` and
`ConfigurationPlugin`. The [FIX registry](../fix/registry.md) owns `MsgType`:
an immutable message Struct definition obtained through registry lookup. Its
wire code stays intact; message definitions have no generic datatype or code
field helper.

## Packed integers and the declared vocabulary

`ascii_packed` is the storage bytes read big-endian: one integer everywhere, ordered as the text, never negative. It answers for a fixed US-ASCII string of at most sixteen bytes or a code, and refuses everything else by name. `StringEnum` names those integers: one US-ASCII value per member, stored on the field under the reserved key `field:enum` ([Protocol](protocol.md)), so the width stays the field's datatype and the enum crosses Arrow, a file, and another runtime intact.

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
        venues.into_members(&DataType::Mic)?,
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
    assert_eq!(Field::from_arrow(&field.into_arrow()?)?.string_enum()?, Some(side.clone()));
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

Python-only enum bases: [Python boundary](../extensions/python.md).

## A state sorts by its lifecycle

`state` is one vocabulary over two worlds: FIX names an order's state twice -
`OrdStatus(39)` says where the order stands and `ExecType(150)` says what the
report is - and a scheduler names a job's state in ordinary English. They are
the same shape, so a capture and the pipeline reading it need one vocabulary
rather than two and a join. The two FIX code sets share their letters and not
always their meaning - `D` is Restated in one and AcceptedForBidding in the
other - so a [FIX column](../fix/capture.md) reads a code through the name its
own field gives it before it reads the letter: `150=D` is `70RESTATED` and
`39=D` is `20ACCEPTED`.

A value is **two decimal digits of rank then a name**, ten US-ASCII bytes. The
rank is what makes the stored bytes sort from the first state to the terminal
ones, and that matters because most things that sort a column are not this
crate: a Parquet row group's bounds, an external sort, an `ORDER BY` in
whatever reads the file. Sorting by name would put `CANCELED` before `NEW`.

Ranks run `00`-`99`. Every shipped state sits on a round rank, and the digits
between two of them - `01`-`09`, `11`-`19`, and so on - are the placeholders a
state that belongs between two ranks takes, so adding one moves nothing
already stored. `State::rank` answers the two digits as the number they spell.

| rank | meaning | members |
| --- | --- | --- |
| `00` | stated, but not a state anything reached | `00UNKNOWN` |
| `10` | asked for, not yet acknowledged | `10PENDING`, `10PENDNEW`, `10QUEUED` |
| `20` | acknowledged, not yet working | `20ACCEPTED`, `20NEW`, `20STARTING`, `20SUBMITTD` |
| `30` | working | `30RUNNING`, `30STATUS`, `30TRIGGER` |
| `40` | working, and something has happened | `40INPROGR`, `40PARTFILL`, `40TRADE`, `40TRDCORR`, `40TRDCXL`, `40TRDHOLD` |
| `50` | halted, and able to resume | `50PAUSED`, `50STOPPED`, `50SUSPEND` |
| `60` | a change is outstanding | `60PENDCXL`, `60PENDRPL` |
| `70` | changed, and the new thing carries on | `70REPLACED`, `70RESTATED` |
| `80` | ended, having done what was asked | `80CALCULAT`, `80COMPLETE`, `80DONEDAY`, `80FILLED`, `80SUCCESS`, `80TRDRELS` |
| `90` | ended, because someone stopped it | `90CANCELED` |
| `95` | ended, because it could not be done | `95EXPIRED`, `95FAILED`, `95REJECTED`, `95TIMEOUT` |

The three endings are ranked apart deliberately: "did it finish" and "did it
work" are different questions, and one terminal rank would answer neither
without reading the name. Each ending owns a band - `80`-`89` done, `90`-`94`
cancelled, `95`-`99` failed - and `State::is_live` (below `80`), `is_done`,
`is_cancelled` and `is_failed` read the band, so a placeholder inside one
answers as its ending does.

Rust only.

```rust
use yggdryl::types::State;

// Four vocabularies reach one value: the wire code an ExecutionReport
// carries, the specification's name for it, a scheduler's word, and the
// short name a FIX bridge logs.
assert_eq!(State::from_spelling("1").unwrap().as_str(), "40PARTFILL");
assert_eq!(State::from_spelling("PartiallyFilled").unwrap().as_str(), "40PARTFILL");
assert_eq!(State::from_spelling("running").unwrap().as_str(), "30RUNNING");
assert_eq!(State::from_spelling("PartFill").unwrap().as_str(), "40PARTFILL");

// The stored bytes sort by lifecycle, which is the whole reason the rank
// leads - nothing but ASCII order is needed to read it back.
let mut held = ["80FILLED", "20NEW", "95REJECTED", "40PARTFILL"];
held.sort_unstable();
assert_eq!(held, ["20NEW", "40PARTFILL", "80FILLED", "95REJECTED"]);

// The rank is a number, and the three endings are told apart by band
// without reading a name.
assert_eq!(State::from_spelling("Filled").unwrap().rank(), Some(80));
assert!(State::from_spelling("New").unwrap().is_live());
assert!(State::from_spelling("Filled").unwrap().is_done());
assert!(State::from_spelling("Rejected").unwrap().is_failed());
```

`timeinforce` is eight bytes over FIX's `TimeInForce(59)` code set, stored as
the wire value rather than a name for it, exactly as `side` is.

## Which way a line moved

`msgdirection` is `SENT` or `RECV`, four bytes. `MsgDirection::from_spelling`
reads a word a caller chose - `sent`, `s`, `send`; `recv`, `r`, `receive`;
`unknown` and the empty text are `None` - folding ASCII case and nothing else.
`MsgDirection::infer_bytes` reads the verb a transport wrote **in front of** a
captured payload, never inside it, and answers nothing where the prefix carries
both verbs or neither. A session's own log is written by the side doing the
sending, so its unmarked lines are `SENT`.

Rust only.

```rust
use yggdryl::types::MsgDirection;

assert_eq!(MsgDirection::from_spelling("Sent")?, Some(MsgDirection::SENT));
assert_eq!(MsgDirection::from_spelling("r")?, Some(MsgDirection::RECV));
assert_eq!(MsgDirection::from_spelling("")?, None);
assert!(MsgDirection::from_spelling("sideways").is_err());
```

## Edges

- A byte past `0x7F`, a NUL, or a value longer than the width -> refused naming the width (`at most 4 bytes`), and the row in a cast.
- Stored under a code -> padded with trailing NUL to the width; every reading trims the padding back. Text carrying trailing NULs canonicalizes to the trimmed value.
- `Scalar::kind()` -> the code's id: `currency`, `msgdirection`, `state`; a plain string's kind is its layout, `string` or `fixed_string`.
- `Code` equality, order and hash carry the identity first, then the text: `Side("1") != TimeInForce("1")`. A code and a plain string of the same bytes are two values.
- `fixed_size_binary(3)` under `yggdryl.currency` -> `currency`; under `yggdryl.string` with a document -> the string it describes; plain -> imports as it is.
- `isin` -> two letters, nine alphanumerics and one digit that closes the eleven before it (ISO 6166's Luhn over the letters expanded to their alphabet positions); a check digit that does not close the number -> refused, `the check digit does not close the number`. Lower case -> the upper case it spells. `Isin::is_valid` and `Isin::closing_digit` answer the rule without building a value.
- An Arrow cast into `isin` is held to the canonical spelling - upper case, closed by its check digit - and refused otherwise, because a column's bytes are what every reader digests; only a scalar read folds the case.
- `isin` names no vocabulary: `StringEnum::from_logical_name("isin")` answers an enum of no members and no Python code class declares it.
- Default value: a code defaults to the empty text its storage does, answered as the code's own scalar. `isin` and `state` are the two exceptions, because their value door gates the space rather than holding it - a check digit closes one and a published vocabulary spells the other - so neither has a neutral member, and `default_value` refuses naming the code rather than answering a value no registry issued.
- A cast refusal under `safe` -> null, which a required column fills with the default; under strict -> the row and the column, for a code exactly as for a string ([Cast](cast.md)).
- [Merged](field.md) widening: a code beside itself -> kept; beside `fixed_ascii(n)`, `ascii` or `utf8` -> that string; beside `fixed_size_binary(n)` of its width -> those bytes.
- [Merged](field.md) narrowing (`upscale=false`): a code beside any plainer shape storing it -> the code; beside narrower text -> that text.
- `currency` beside `country` -> `fixed_ascii(3)` widening and `fixed_ascii(2)` narrowing, the plain text both fit, never one code holding the other's values.
- Iceberg, Spark, Polars, pandas, Avro, filter literals -> text, [rewritten](datatype.md) to `string`/`utf8`. Every registered code, not a subset: the listing each of these paths reads is `DataType::CODES`, through `DataType::is_code` and `DataType::fixed_byte_width`, so a code cannot be spellable in one and unspellable in the next.
- An Arrow cast into any code -> one fixed binary column padded to the code's own width, from a fixed binary source of that width or from anything Arrow renders as text; anything else -> refused naming the code and the source.
- `ascii_packed` on a variable string, on UTF-8, or on a width past 16 bytes -> refused, `at most 16 bytes`.
- `ascii_packed` -> an `i32`, an `i64`, or a whole `i128` by width, and the integer a stable hash hashes.
- A `StringEnum` on a string that is not fixed US-ASCII of at most sixteen bytes -> refused by name at `set_string_enum` and `into_members`; error kind `string-enum`.
- `field:enum` document -> the width never enters it, so one enum is one canonical text.
- `from_logical_name` -> the shipped `COUNTRIES`, `CURRENCIES`, `MICS`, `SIDES`, `DIRECTIONS`, `STATES`, `TIMESINFORCE` listings, `prebuilt()` in either binding; `"Exchange"` -> `MICS`.
- `from_logical_name("tenor")` (registered, no listing) -> an empty enum.
- JavaScript `readRecords` -> Arrow JS rows carry no extension identity, so a code column arrives as stored bytes; declare `utf8` to read text.
- A wire code never folds: `A` is `PendingNew` and `a` names no state, because they are different FIX codes and a folded lookup would answer the wrong state for one of them.
- A name folds: `DoneForDay`, `done_for_day`, `DONE FOR DAY` and a bridge's `DoneDay` are one spelling, `80DONEDAY`.
- `State::rank` on a value that does not open with two digits -> `None`, and every lifecycle predicate answers `false`.
- A spelling nothing publishes answers nothing rather than a guess.
- Digests: a code value feeds its own id as the tag, so a stored digest of a code cell differs from the same bytes under `fixed_ascii(n)`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::ascii datatype::coded field::ascii
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- types::string types::tests::string_enum types::tests::vocabulary
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^ascii/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "registered_code or prebuilt_vocabulary or enum_member or code_datatype"
    python/.venv/bin/python -m pytest python/tests/test_enums.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code|vocabulary|enum|packs" node/tests/types/datatype.test.js node/tests/types/fields.test.js
    npm run --prefix node bench:types
    ```
