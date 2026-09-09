# ASCII

Variable and fixed-width ASCII text, the four registered codes, packed integers, and the `AsciiEnum` vocabulary a field declares.

## Contract

| Spelling | Width | Arrow storage, extension |
| --- | ---: | --- |
| `ascii` | none | `binary`, `yggdryl.ascii` |
| `ascii(n)` | `n` | `fixed_size_binary(n)`, `yggdryl.ascii` |
| `country`, ISO 3166-1 alpha-2 | 2 | `fixed_size_binary(2)`, `yggdryl.country` |
| `currency`, ISO 4217 | 3 | `fixed_size_binary(3)`, `yggdryl.currency` |
| `mic`, ISO 10383 | 4 | `fixed_size_binary(4)`, `yggdryl.mic` |
| `cfi`, ISO 10962 | 6 | `fixed_size_binary(6)`, `yggdryl.cfi` |

## Use

The [playground](playground.md) renders every width, code, refusal, and vocabulary; `CODES` lists the codes, `is_code` and `code_name` tell one from a width.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, RecordBatch, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{ArrowCast, DataType, DataTypeKind, Field, Scalar};

    // Two shapes: text of any length, and text padded to one fixed width.
    assert_eq!(DataType::from_str("ascii")?, DataType::Ascii);
    assert_eq!(DataType::ascii(3)?, DataType::FixedAscii(3));
    assert_eq!(DataType::from_str("ascii(12)")?, DataType::FixedAscii(12));
    assert_eq!(DataType::FixedAscii(4).to_string(), "ascii(4)");
    assert_eq!(DataType::Ascii.kind(), DataTypeKind::Ascii);
    assert_eq!(DataType::FixedAscii(8).ascii_width(), Some(8));
    // Variable ASCII has no width to report, and neither has anything else.
    assert_eq!(DataType::Ascii.ascii_width(), None);
    assert_eq!(DataType::Utf8.ascii_width(), None);
    assert!(DataType::ascii(0).is_err());

    // A registered code is a datatype, not a name over a width: it stores the
    // width its standard fixes and displays as itself.
    let currency = DataType::currency();
    assert_eq!(DataType::from_str("currency")?, currency);
    assert_eq!(currency.to_string(), "currency");
    assert_eq!(currency.ascii_width(), Some(3));
    assert_ne!(currency, DataType::FixedAscii(3));
    assert_eq!(currency.kind(), DataTypeKind::Ascii);
    assert_eq!(
        DataType::CODES,
        &[
            ("country", DataType::Country, 2),
            ("currency", DataType::Currency, 3),
            ("mic", DataType::Mic, 4),
            // Six bytes: `cfi` stores what it is, not the eight some other
            // width would pad it to.
            ("cfi", DataType::Cfi, 6),
            // FIX-facing codes retain their individual storage widths.
            ("side", DataType::Side, 4),
            ("msgdirection", DataType::MsgDirection, 4),
            ("state", DataType::State, 8),
            ("timeinforce", DataType::TimeInForce, 8),
        ]
    );

    // A code rides its own Arrow extension, so the identity survives the trip.
    let venue = Field::new("venue", DataType::Mic, false);
    let arrow = venue.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(4));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.mic");
    assert_eq!(Field::from_arrow(&arrow)?, venue);

    // Storage pads to the width; every string rendering trims the padding.
    let ccy = Field::new("ccy", DataType::FixedAscii(4), false);
    let stored = scalar_array(&ccy, &Scalar::from("USD"))?;
    let bytes = stored.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(bytes.value(0), b"USD\0");
    assert_eq!(
        scalar_value(&ccy, stored.as_ref())?,
        DataType::FixedAscii(4).scalar("USD")?
    );

    // The Arrow field is `fixed_size_binary(4)` under the `yggdryl.ascii` name.
    let arrow = ccy.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(4));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.ascii");
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
    assert_eq!(Field::from_arrow(&arrow)?, ccy);

    // The variable form is the same extension over Arrow's `Binary`: no width,
    // so no padding, and the storage is the bytes the value is.
    let note = Field::new("note", DataType::Ascii, false);
    let arrow = note.clone().into_arrow()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Binary);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.ascii");
    assert_eq!(Field::from_arrow(&arrow)?, note);
    let free = scalar_array(&note, &Scalar::from("a note of any length at all"))?;
    let free = free.as_any().downcast_ref::<BinaryArray>().unwrap();
    assert_eq!(free.value(0), b"a note of any length at all");

    // A cast into the width pads; the stored column read under `utf8` trims.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EU"]));
    let exact = yggdryl::ArrowCastOptions::new().with_safe(false);
    let padded = ccy.cast_arrow_array(text, exact)?;
    let bytes = padded.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(bytes.value(1), b"EU\0\0");
    let row = DataType::from_fields([ccy.clone()])?.required_field("row");
    let batch = RecordBatch::try_new(row.into_arrow_schema()?, vec![padded])?;
    let text = DataType::from_fields([DataType::Utf8.required_field("ccy")])?.required_field("row");
    let trimmed = text.cast_arrow_batch(batch, exact)?;
    let trimmed = trimmed.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(trimmed.value(1), "EU");

    // A width merged with the variable form drops the width, and either
    // merged with text is text.
    assert_eq!(
        DataType::FixedAscii(4).merge_with(&DataType::Ascii, true)?,
        DataType::Ascii
    );
    assert_eq!(DataType::Ascii.merge_with(&DataType::Utf8, true)?, DataType::Utf8);

    let long: ArrayRef = Arc::new(StringArray::from(vec!["EURO!"]));
    let refused = ccy.cast_arrow_array(long, exact).unwrap_err().to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import DataType, Field, types

    # Two shapes: text of any length, and text padded to one fixed width.
    note = DataType("ascii")
    ascii32 = DataType.ascii(4)
    assert note.id == "ascii"
    assert ascii32.id == "fixed_ascii"
    assert str(ascii32) == "ascii(4)"
    assert DataType("ascii(12)") == DataType.ascii(12)
    assert ascii32.kind == note.kind == "ascii"
    assert ascii32.ascii_width == 4
    assert DataType.ascii(12).ascii_width == 12
    # Variable ASCII has no width to report, and neither has anything else.
    assert note.ascii_width is None
    assert DataType("utf8").ascii_width is None
    assert types.fixed_ascii("ccy", 3).dtype == DataType.ascii(3)
    assert types.ascii("note").dtype == note

    # A registered code is a datatype, not a name over a width: it stores the
    # width its standard fixes and displays as itself.
    currency = DataType("currency")
    assert str(currency) == "currency"
    assert currency.ascii_width == 3
    assert currency != DataType.ascii(3)
    assert currency.kind == "ascii"
    assert [(DataType(name).id, DataType(name).ascii_width) for name in
            ("country", "currency", "mic", "cfi")] == [
        ("country", 2), ("currency", 3), ("mic", 4), ("cfi", 6)
    ]

    # A code rides its own Arrow extension, so the identity survives the trip.
    venue = types.mic("venue", nullable=False)
    venue_arrow = venue.into_arrow()
    assert venue_arrow.type == pa.binary(4)
    assert venue_arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.mic"
    assert Field.from_arrow(venue_arrow) == venue

    # Storage pads to the width; every string rendering trims the padding.
    ccy = Field("ccy", ascii32, nullable=False)
    assert ccy.arrow_scalar("USD") == pa.scalar(b"USD\x00", pa.binary(4))
    assert ccy.default_scalar().as_py() == ""

    # The Arrow field is `fixed_size_binary(4)` under the `yggdryl.ascii` name.
    arrow = ccy.into_arrow()
    assert arrow.type == pa.binary(4)
    assert arrow.metadata == {
        b"ARROW:extension:name": b"yggdryl.ascii",
        b"ARROW:extension:metadata": b"",
    }
    assert Field.from_arrow(arrow) == ccy

    # The variable form is the same extension over Arrow's variable binary: no
    # width, so no padding, and the storage is the bytes the value is.
    free = types.ascii("note", nullable=False)
    free_arrow = free.into_arrow()
    assert free_arrow.type == pa.binary()
    assert free_arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.ascii"
    assert Field.from_arrow(free_arrow) == free
    assert free.arrow_scalar("a note of any length at all") == pa.scalar(
        b"a note of any length at all", pa.binary()
    )

    # A cast into the width pads; the stored column read under `utf8` trims.
    padded = ccy.cast_arrow_array(pa.array(["USD", "EU"]))
    assert padded.to_pylist() == [b"USD\x00", b"EU\x00\x00"]
    stored = pa.record_batch([padded], schema=pa.schema([arrow]))
    text = DataType.from_fields([types.utf8("ccy")])
    assert text.cast_arrow_batch(stored).column(0).to_pylist() == ["USD", "EU"]

    # A width merged with the variable form drops the width, and either merged
    # with text is text.
    assert ascii32.merge_with(note) == note
    assert note.merge_with("utf8") == DataType("utf8")

    with pytest.raises(ValueError, match="at most 4 bytes"):
        ccy.cast_arrow_array(pa.array(["EURO!"]))
    with pytest.raises(ValueError, match="at least 1 byte, got 0"):
        DataType.ascii(0)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { DataType, fields } = require('yggdryl')

    // Two shapes: text of any length, and text padded to one fixed width.
    const note = new DataType('ascii')
    const ascii32 = DataType.ascii(4)
    assert.equal(note.id, 'ascii')
    assert.equal(ascii32.id, 'fixed_ascii')
    assert.equal(ascii32.toString(), 'ascii(4)')
    assert.ok(DataType.from('ascii(12)').equals(DataType.ascii(12)))
    assert.equal(ascii32.kind, 'ascii')
    assert.equal(ascii32.asciiWidth, 4)
    // Variable ASCII has no width to report, and neither has anything else.
    assert.equal(note.asciiWidth, null)
    assert.equal(new DataType('utf8').asciiWidth, null)
    assert.ok(fields.fixedAscii('ccy', 3).dtype.equals(DataType.ascii(3)))
    assert.equal(fields.ascii('note').dtype.id, 'ascii')

    // A registered code is a datatype, not a name over a width: it stores the
    // width its standard fixes and displays as itself.
    const currency = new DataType('currency')
    assert.equal(currency.id, 'currency')
    assert.equal(currency.toString(), 'currency')
    assert.equal(currency.asciiWidth, 3)
    assert.ok(!currency.equals(DataType.ascii(3)))
    assert.deepEqual(
      ['country', 'currency', 'mic', 'cfi'].map((name) => new DataType(name).asciiWidth),
      [2, 3, 4, 6],
    )

    // A code rides its own Arrow extension, so the identity survives the trip.
    const venue = fields.struct('row', [fields.mic('venue', { nullable: false })], {
      nullable: false,
    })
    const venueArrow = venue.castArrow(
      new arrow.Table({ venue: arrow.vectorFromArray(['XPAR'], new arrow.Utf8()) }),
    ).schema.fields[0]
    assert.equal(String(venueArrow.type), 'FixedSizeBinary[4]')
    assert.equal(venueArrow.metadata.get('ARROW:extension:name'), 'yggdryl.mic')

    // Storage pads to the width; every string rendering trims the padding.
    const row = fields.struct('row', [fields.fixedAscii('ccy', 4, { nullable: false })], {
      nullable: false,
    })
    assert.equal(row.getField('ccy').defaultJSValue(), '')
    const codes = (values) =>
      new arrow.Table({ ccy: arrow.vectorFromArray(values, new arrow.Utf8()) })
    const stored = row.castArrow(codes(['USD', 'EU']))
    assert.deepEqual([...stored.getChild('ccy').get(1)], [0x45, 0x55, 0, 0])

    // The Arrow field is `FixedSizeBinary[4]` under the `yggdryl.ascii` name,
    // and a column carrying that identity reads under `utf8` as trimmed text.
    const field = stored.schema.fields[0]
    assert.equal(String(field.type), 'FixedSizeBinary[4]')
    assert.equal(field.metadata.get('ARROW:extension:name'), 'yggdryl.ascii')
    const text = fields.struct('row', [fields.utf8('ccy', { nullable: false })], {
      nullable: false,
    })
    assert.deepEqual([...text.castArrow(stored).getChild('ccy')], ['USD', 'EU'])

    // The variable form is the same extension over Arrow's `Binary`: no width,
    // so no padding, and the storage is the bytes the value is.
    const notes = fields.struct('row', [fields.ascii('note', { nullable: false })], {
      nullable: false,
    })
    const free = notes.castArrow(
      new arrow.Table({
        note: arrow.vectorFromArray(['a note of any length at all'], new arrow.Utf8()),
      }),
    )
    assert.equal(String(free.schema.fields[0].type), 'Binary')
    assert.equal(
      Buffer.from(free.getChild('note').get(0)).toString(),
      'a note of any length at all',
    )

    // A width merged with the variable form drops the width, and either merged
    // with text is text.
    assert.equal(ascii32.mergeWith(note).id, 'ascii')
    assert.equal(note.mergeWith('utf8').id, 'utf8')

    assert.throws(() => row.castArrow(codes(['EURO!'])), /ASCII text of at most 4 bytes/)
    assert.throws(() => DataType.ascii(0), /at least 1 byte, got 0/)
    ```

## FIX message definitions

FIX tag 35 and the [capture `msgtype` column](../media/text.md#classifying-each-record)
store complete `utf8` text, including codes such as `P Report Ack` and
`ConfigurationPlugin`. The [FIX registry](../fix/registry.md) owns `MsgType`:
an immutable message Struct definition obtained through registry lookup. Its
wire code stays intact; message definitions have no generic datatype or ASCII
field helper.

## Declared vocabulary and generated enum

`ascii_packed` is the storage bytes read big-endian: one integer everywhere, ordered as the text, never negative.

=== "Rust"

    ```rust
    use yggdryl::{AsciiEnum, DataType, Field};

    // A value's integer is its own storage bytes read big-endian, so it is the
    // same integer in every process and orders exactly as the text does.
    assert_eq!(DataType::FixedAscii(4).ascii_packed(b"USD")?, 0x5553_4400);
    assert_eq!(DataType::FixedAscii(4).ascii_packed(b"USD\0")?, 0x5553_4400);
    assert_eq!(DataType::FixedAscii(4).ascii_value(0x5553_4400)?, "USD");
    assert_eq!(DataType::Currency.ascii_packed(b"USD")?, 0x0055_5344);
    // Sixteen bytes fill the whole `i128`; a wider width has no packed code.
    assert_eq!(
        DataType::FixedAscii(16).ascii_packed(b"US0378331005")?,
        0x5553_3033_3738_3333_3130_3035_0000_0000
    );
    assert!(DataType::FixedAscii(17).ascii_packed(b"US").is_err());
    assert!(DataType::Ascii.ascii_packed(b"US").is_err());

    // An enum is that naming as a value: one ASCII value per member name.
    let venues = AsciiEnum::from_members("Venue", [("XNAS", "XNAS"), ("N_A", "n/a")])?;
    assert_eq!(venues.get("N_A"), Some("n/a"));
    assert_eq!(
        venues.into_members(&DataType::Mic)?,
        [("N_A".into(), 0x6E2F_6100), ("XNAS".into(), 0x584E_4153)]
    );

    // The same rule names one value at a time, for a vocabulary declared
    // member by member rather than generated from a whole listing.
    assert_eq!(AsciiEnum::member_name("n/a").as_str(), "N_A");

    // The ISO listings ship with the package, so a code column declares the
    // vocabulary it draws from without a copy per language.
    let currencies = AsciiEnum::from_logical_name("currency")?;
    assert_eq!(currencies.len(), AsciiEnum::CURRENCIES.len());
    assert_eq!(currencies.get("USD"), Some("USD"));
    assert_eq!(AsciiEnum::from_logical_name("Exchange")?.len(), AsciiEnum::MICS.len());
    // A registered name with no listing answers an enum of no members.
    assert!(AsciiEnum::from_logical_name("tenor")?.is_empty());

    // A field declares the enum its values name, as one metadata document, so
    // the enum crosses Arrow and comes back the enum that was written.
    let side = AsciiEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")])?;
    let field = Field::new("side", DataType::FixedAscii(4), false).try_with_ascii_enum(&side)?;
    assert_eq!(side.into_members(&DataType::FixedAscii(4))?[0], ("BUY".into(), 0x4200_0000));
    assert_eq!(Field::from_arrow(&field.into_arrow()?)?.ascii_enum()?, Some(side));
    ```

=== "Python"

    ```python
    import enum

    import pytest

    from yggdryl import AsciiEnum, DataType, Field

    # A value's integer is its own storage bytes read big-endian, so it is the
    # same integer in every process and orders exactly as the text does.
    ascii32 = DataType.ascii(4)
    assert ascii32.ascii_packed("USD") == 0x55534400
    assert ascii32.ascii_packed("USD\x00") == 0x55534400
    assert ascii32.ascii_value(0x55534400) == "USD"
    assert DataType("currency").ascii_packed("USD") == 0x555344
    # Sixteen bytes fill the whole 128-bit integer, which Python holds natively.
    assert DataType.ascii(16).ascii_packed("US0378331005") == (
        0x55533033373833333130303500000000
    )
    with pytest.raises(ValueError, match="at most 16 bytes"):
        DataType("ascii").ascii_packed("US")

    # An enum is that naming as a value: one ASCII value per member name.
    venues = AsciiEnum("Venue", {"XNAS": "XNAS", "N_A": "n/a"})
    assert venues.get("N_A") == "n/a"
    assert venues.into_members("mic") == [("N_A", 0x6E2F6100), ("XNAS", 0x584E4153)]

    # ... and as a Python `IntEnum`, keyed by the same integers.
    Venue = venues.into_intenum("mic")
    assert issubclass(Venue, enum.IntEnum)
    assert Venue(0x584E4153).name == "XNAS"

    # The same rule names one value at a time, for a vocabulary declared
    # member by member rather than generated from a whole listing.
    assert AsciiEnum.member_name("n/a") == "N_A"

    # The ISO listings ship with the package, so a code column declares the
    # vocabulary it draws from without a copy per language.
    currencies = AsciiEnum.from_logical_name("currency")
    assert len(currencies) == len(AsciiEnum.prebuilt()["currency"])
    assert currencies.get("USD") == "USD"
    assert len(AsciiEnum.from_logical_name("Exchange")) == len(AsciiEnum.prebuilt()["mic"])
    # A registered name with no listing answers an enum of no members.
    assert len(AsciiEnum.from_logical_name("tenor")) == 0

    # A field declares the enum its values name, as one metadata document, so
    # the enum crosses Arrow and comes back the enum that was written.
    side = AsciiEnum("Side", {"BUY": "B", "SELL": "S"})
    field = Field("side", ascii32, nullable=False)
    field.set_ascii_enum(side)
    assert side.into_members(ascii32)[0] == ("BUY", 0x42000000)
    assert Field.from_arrow(field.into_arrow()).ascii_enum == side
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { AsciiEnum, DataType, Field } = require('yggdryl')

    // A value's integer is its own storage bytes read big-endian, so it is the
    // same integer in every process and orders exactly as the text does.
    const ascii32 = DataType.ascii(4)
    assert.equal(ascii32.asciiPacked('USD'), 0x55534400n)
    assert.equal(ascii32.asciiPacked('USD\0'), 0x55534400n)
    assert.equal(ascii32.asciiValue(0x55534400n), 'USD')
    assert.equal(new DataType('currency').asciiPacked('USD'), 0x555344n)
    // Sixteen bytes fill the whole 128-bit integer, so every code is a bigint.
    assert.equal(
      DataType.ascii(16).asciiPacked('US0378331005'),
      0x55533033373833333130303500000000n,
    )
    assert.throws(() => new DataType('ascii').asciiPacked('US'), /at most 16 bytes/)

    // An enum is that naming as a value: one ASCII value per member name.
    const venues = new AsciiEnum('Venue', { XNAS: 'XNAS', N_A: 'n/a' })
    assert.equal(venues.get('N_A'), 'n/a')
    assert.deepEqual(venues.intoMembers('mic'), { XNAS: 0x584e4153n, N_A: 0x6e2f6100n })

    // ... and as the generated enum: a frozen name-to-code object, tagged with
    // the enum's own name.
    const Venue = venues.intoEnum('mic')
    assert.equal(Venue.XNAS, new DataType('mic').asciiPacked('XNAS'))
    assert.equal(Object.prototype.toString.call(Venue), '[object Venue]')

    // The same rule names one value at a time, for a vocabulary declared
    // member by member rather than generated from a whole listing.
    assert.equal(AsciiEnum.memberName('n/a'), 'N_A')

    // The ISO listings ship with the package, so a code column declares the
    // vocabulary it draws from without a copy per language.
    const currencies = AsciiEnum.fromLogicalName('currency')
    assert.equal(currencies.length, AsciiEnum.prebuilt().currency.length)
    assert.equal(currencies.get('USD'), 'USD')
    assert.equal(AsciiEnum.fromLogicalName('tenor').length, 0)

    // A field declares the enum its values name, as one metadata document, so
    // every serialization carries it and it comes back the enum that wrote it.
    const side = new AsciiEnum('Side', { BUY: 'B', SELL: 'S' })
    const field = new Field('side', ascii32, false)
    field.setAsciiEnum(side)
    assert.deepEqual(side.intoMembers(ascii32), { BUY: 0x42000000n, SELL: 0x53000000n })
    assert.ok(Field.fromJSON(field.toJSON()).asciiEnum.equals(side))
    ```

`AsciiEnum` is an enum name plus one ASCII value per member, stored under the reserved key `field:enum` ([Protocol](protocol.md)); the width stays the field's datatype.

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
rather than two and a join.

A value is a **rank character then a name**, eight ASCII bytes. The rank is
what makes the stored bytes sort from the first state to the terminal ones,
and that matters because most things that sort a column are not this crate: a
Parquet row group's bounds, an external sort, an `ORDER BY` in whatever reads
the file. Sorting by name would put `CANCELD` before `NEW`.

| rank | meaning | members |
| --- | --- | --- |
| `0` | stated, but not a state anything reached | `0UNKNOWN` |
| `1` | asked for, not yet acknowledged | `1PENDING`, `1PENDNEW`, `1QUEUED` |
| `2` | acknowledged, not yet working | `2ACCEPTD`, `2NEW`, `2STARTNG`, `2SUBMITD` |
| `3` | working | `3RUNNING`, `3STATUS`, `3TRIGGER` |
| `4` | working, and something has happened | `4INPROGR`, `4PARTFIL`, `4TRADE`, `4TRDCORR`, `4TRDCXL`, `4TRDHOLD` |
| `5` | halted, and able to resume | `5PAUSED`, `5STOPPED`, `5SUSPEND` |
| `6` | a change is outstanding | `6PENDCXL`, `6PENDRPL` |
| `7` | changed, and the new thing carries on | `7REPLACD` |
| `8` | ended, having done what was asked | `8CALCULD`, `8COMPLET`, `8DONEDAY`, `8FILLED`, `8SUCCESS`, `8TRDRELS` |
| `9` | ended, because someone stopped it | `9CANCELD` |
| `A` | ended, because it could not be done | `AEXPIRED`, `AFAILED`, `AREJECTD`, `ATIMEOUT` |

The three endings are ranked apart deliberately: "did it finish" and "did it
work" are different questions, and one terminal rank would answer neither
without reading the name.

=== "Rust"

    ```rust
    use yggdryl::types::State;

    // Three vocabularies reach one value: the wire code an ExecutionReport
    // carries, the specification's name for it, and a scheduler's word.
    assert_eq!(State::from_spelling("1").unwrap().as_str(), "4PARTFIL");
    assert_eq!(State::from_spelling("PartiallyFilled").unwrap().as_str(), "4PARTFIL");
    assert_eq!(State::from_spelling("running").unwrap().as_str(), "3RUNNING");

    // The stored bytes sort by lifecycle, which is the whole reason the rank
    // leads - nothing but ASCII order is needed to read it back.
    let mut held = ["8FILLED", "2NEW", "AREJECTD", "4PARTFIL"];
    held.sort_unstable();
    assert_eq!(held, ["2NEW", "4PARTFIL", "8FILLED", "AREJECTD"]);

    // And the three endings are told apart without reading a name.
    assert!(State::from_spelling("New").unwrap().is_live());
    assert!(State::from_spelling("Filled").unwrap().is_done());
    assert!(State::from_spelling("Rejected").unwrap().is_failed());
    ```

`timeinforce` is the same eight bytes over FIX's `TimeInForce(59)` code set,
stored as the wire value rather than a name for it, exactly as `side` is.

## Edges

- `ascii(0)` -> refused, `at least 1 byte, got 0`.
- A byte past `0x7F`, a NUL, or a value longer than the width -> refused naming the width (`at most 4 bytes`), and the row in a cast.
- Stored under `ascii(n)` -> padded with trailing NUL to `n`; every string rendering trims the padding back.
- Canonical scalar -> the trimmed string; bytes and text carrying trailing NULs are accepted and canonicalize to it.
- `fixed_size_binary(3)` under `yggdryl.currency` -> `currency`; under `yggdryl.ascii` -> `ascii(3)`; plain, or carrying a document -> imports as it is.
- [Merged](field.md) widening: a code beside itself -> kept; a width beside `ascii` -> `ascii`; either beside `utf8` -> `utf8`; a code beside `fixed_size_binary(n)` of its width -> those bytes.
- [Merged](field.md) narrowing (`upscale=false`): a code beside any plainer shape storing it - `ascii(n)`, `ascii`, `utf8`, `fixed_size_binary(n)` -> the code; beside narrower text -> that text.
- `currency` beside `country` -> `ascii(3)` widening and `ascii(2)` narrowing, the plain text both fit, never one code holding the other's values.
- Iceberg, Spark, Polars, pandas, Avro, filter literals -> text, [rewritten](datatype.md) to `string`/`utf8`.
- `ascii_packed` on `ascii`, or on a width past 16 bytes -> refused, `at most 16 bytes`.
- `ascii_packed` -> an `i32`, an `i64`, or a whole `i128` by width, and the integer a stable hash hashes.
- `field:enum` document -> the width never enters it, so one enum is one canonical text.
- `from_logical_name` -> the shipped `COUNTRIES`, `CURRENCIES`, `MICS` listings, `prebuilt()` in either binding; `"Exchange"` -> `MICS`.
- `from_logical_name("tenor")` (registered, no listing) -> an empty enum.
- JavaScript `readRecords` -> Arrow JS rows carry no extension identity, so an ASCII column arrives as stored bytes; declare `utf8` to read text.
- A wire code never folds: `A` is `PendingNew` and `a` names no state.
- A stored value names itself, so resolving one twice is resolving it once.
- A spelling nothing publishes answers nothing rather than a guess.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::ascii datatype::coded field::ascii
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- types::ascii types::tests::ascii types::tests::ascii_enum
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^ascii/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "ascii or registered_code or prebuilt_vocabulary or enum_member"
    python/.venv/bin/python -m pytest python/tests/test_enums.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="ASCII|ascii|registered code|vocabulary|enum" node/tests/types/datatype.test.js node/tests/types/fields.test.js
    npm run --prefix node bench:types
    ```
