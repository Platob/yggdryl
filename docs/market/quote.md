# Quote

`market::Quote` is a price stated at an instant: one or two lanes, each a price, a size, a currency and a unit, under the quote's own identifiers and its validity - read out of the quote message, its status reports and its cancel, one statement per message, chained.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `rust/src/market/quote.rs`: the `Quote` trait, the `QuoteData` holder, the row and the `Product` answers; the FIX reading is `FixCodec::quotes` beside the [codec](../fix/index.md) |
| Chain | the quote's: `crosscode` is `QuoteID(117)`, else `QuoteReqID(131)`, and `crossuuid` the identity it derives; the names it goes by are `quoteid`, `quotereqid`, `secondaryquoteid`, `quotemsgid` and `quoteentryid` |
| Facts | the bid lane - `bidpx`, `bidqty`, `bidcurrency`, `bidunit` - what the quoter would pay, and the ask lane what it would be paid, one of them empty on a one-sided quote; a lane priced in no currency of its own is priced in the quote's, `Currency(15)`, and counted in its unit; the ticker and the six codes; `expirunix` how long the quote stands, `ValidUntilTime(62)`. Exactly what the traits name, so it holds a [`MarketEventData`](../graph.md) and nothing beside |
| Reads | `bid()` and `ask()` answer a lane only where both its price and its size are stated above zero - a zero price or size is no quote on that side, which is FIX's own rule and what a withdrawn lane looks like; `lane(side)` answers the lane a side takes, nothing for one that takes none; `is_two_sided()`; and `mid()` and `spread()` read across the two tops |
| State | a quote cancel (`35=Z`) ends the chain with `90CANCELED`; every other statement stands where the message says, `00UNKNOWN` where it says nothing |
| Doors | `FixCodec::quotes(messages)` over a quote (`35=S`), a quote status report (`AI`) and a quote cancel (`Z`); `quotes_arrow_reader(source)` under `QuoteData::field()` |
| Writes back | refused: `FixMsg::from_quote` answers the one sentence - a quote is a reading of the quote message and its updates, and no one message states which of them a reading came from, so the crate does not guess |
| Bindings | Rust `Quote`/`QuoteData`; Python `yggdryl.market.Quote`, `FixCodec.quotes`, `quotes_arrow_reader`, `FixMsg.from_quote`; JavaScript `market.Quote`, `quotes`, `quotesArrowReader`, `FixMsg.fromQuote` |

## Use

A two-sided quote, its update and its cancel, then a one-sided quote of another identifier.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::local::Folder;
    use yggdryl::market::{Product, Quote, QuoteData};
    use yggdryl::{Decimal18, FixCodec, FixMsg, FixRegistry, Side};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=S|117=Q1|131=R1|55=AAPL|132=10.4|134=100|133=10.6|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=100|133=10.55|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:31.250|10=0|",
        b"8=FIX.4.4|35=Z|117=Q1|298=1|52=20260102-10:15:32.250|10=0|",
        b"8=FIX.4.4|35=S|117=Q2|55=AAPL|54=1|132=10.3|134=50|15=USD|52=20260102-10:15:33.250|10=0|",
    ];
    let quotes: Vec<QuoteData> = reader.quotes(reader.parse_lines(lines)).collect::<yggdryl::Result<_>>()?;
    let [first, update, cancel, one_sided] = quotes.as_slice() else { panic!("four statements") };
    // The lanes, and what they imply.
    assert_eq!(first.bid(), Some(("10.4".parse()?, Decimal18::from_int(100))));
    assert_eq!(first.ask(), Some(("10.6".parse()?, Decimal18::from_int(150))));
    assert!(first.is_two_sided());
    assert_eq!(first.mid(), Some("10.5".parse()?));
    assert_eq!(first.spread(), Some("0.2".parse()?));
    assert_eq!(first.lane(&Side::read("Sell")?), first.ask());
    assert_eq!(first.lane(&Side::read("Cross")?), None, "a cross takes no lane");
    assert_eq!(first.get_bidcurrency().map(yggdryl::Currency::as_str), Some("USD"), "priced in the quote's currency");
    assert_eq!(first.get_expirunix(), Some(1_767_348_960_000_000_000));
    assert_eq!((first.get_crosscode(), &first.get_identifiers()["quotereqid"]), ("Q1", &"R1".to_owned()));
    // The update follows under the quote's identifier, and the cancel ends the chain.
    assert_eq!(update.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(update.get_bidpx(), Some("10.45".parse()?));
    assert_eq!((cancel.get_seqnum(), cancel.get_state().as_str()), (2, "90CANCELED"));
    assert!(!cancel.is_alive() && cancel.bid().is_none());
    // A one-sided quote quotes one lane, and is about no mid.
    assert_eq!(one_sided.bid(), Some(("10.3".parse()?, Decimal18::from_int(50))));
    assert!(!one_sided.is_two_sided());
    assert_eq!((one_sided.mid(), one_sided.spread()), (None, None));
    assert_eq!(one_sided.get_px(), "10.3".parse()?, "about the bid it states");
    // No one message states a quote.
    assert!(FixMsg::from_quote(&reader, first).unwrap_err().to_string().contains("does not guess"));
    let field = QuoteData::field()?;
    assert_eq!(field.fields()[16].name(), "bidpx");
    assert_eq!(QuoteData::from_row(&field, &first.into_row()?)?.get_curruuid(), first.get_curruuid());
    ```

=== "Python"

    ```python
    from decimal import Decimal
    from pathlib import Path

    import pytest

    from yggdryl.fix import FixCodec, FixMsg, FixRegistry
    from yggdryl.market import Quote

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    lines = [
        b"8=FIX.4.4|35=S|117=Q1|131=R1|55=AAPL|132=10.4|134=100|133=10.6|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=100|133=10.55|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:31.250|10=0|",
        b"8=FIX.4.4|35=Z|117=Q1|298=1|52=20260102-10:15:32.250|10=0|",
        b"8=FIX.4.4|35=S|117=Q2|55=AAPL|54=1|132=10.3|134=50|15=USD|52=20260102-10:15:33.250|10=0|",
    ]
    first, update, cancel, one_sided = reader.quotes(reader.parse_lines(lines))
    # The lanes, and what they imply.
    lane = lambda held: tuple(cell.as_py() for cell in held)  # a lane is two Scalars
    assert lane(first.bid) == (Decimal("10.4"), Decimal("100"))
    assert lane(first.ask) == (Decimal("10.6"), Decimal("150"))
    assert first.is_two_sided
    assert first.mid.as_py() == Decimal("10.5")
    assert first.spread.as_py() == Decimal("0.2")
    assert first.lane("Sell") == first.ask
    assert first.lane("Cross") is None  # a cross takes no lane
    assert first.bidcurrency.as_py() == "USD"  # priced in the quote's currency
    assert first.expirunix == 1_767_348_960_000_000_000
    assert (first.crosscode, first.identifiers["quotereqid"]) == ("Q1", "R1")
    # The update follows under the quote's identifier, and the cancel ends the chain.
    assert update.prevuuid == first.curruuid
    assert update.bidpx.as_py() == Decimal("10.45")
    assert (cancel.seqnum, cancel.state.as_py()) == (2, "90CANCELED")
    assert not cancel.is_alive and cancel.bid is None
    # A one-sided quote quotes one lane, and is about no mid.
    assert lane(one_sided.bid) == (Decimal("10.3"), Decimal("50"))
    assert not one_sided.is_two_sided
    assert one_sided.mid is None and one_sided.spread is None
    assert one_sided.event().px.as_py() == Decimal("10.3")  # about the bid it states
    # No one message states a quote.
    with pytest.raises(ValueError, match="does not guess"):
        FixMsg.from_quote(reader, first)
    field = Quote.field()
    assert field.field_at(16).name == "bidpx"
    assert Quote.from_row(field, first.into_row()).curruuid == first.curruuid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix, market } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)
    const lines = [
      '8=FIX.4.4|35=S|117=Q1|131=R1|55=AAPL|132=10.4|134=100|133=10.6|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:30.250|10=0|',
      '8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=100|133=10.55|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:31.250|10=0|',
      '8=FIX.4.4|35=Z|117=Q1|298=1|52=20260102-10:15:32.250|10=0|',
      '8=FIX.4.4|35=S|117=Q2|55=AAPL|54=1|132=10.3|134=50|15=USD|52=20260102-10:15:33.250|10=0|',
    ].map((line) => Buffer.from(line))
    const [first, update, cancel, oneSided] = [...reader.quotes(reader.parseLines(lines))]
    // The lanes, and what they imply.
    assert.deepEqual(first.bid, { px: '10.4', qty: '100' })
    assert.deepEqual(first.ask, { px: '10.6', qty: '150' })
    assert.equal(first.isTwoSided, true)
    assert.equal(first.mid, '10.5')
    assert.equal(first.spread, '0.2')
    assert.deepEqual(first.lane('Sell'), first.ask)
    assert.equal(first.lane('Cross'), null) // a cross takes no lane
    assert.equal(first.bidcurrency, 'USD') // priced in the quote's currency
    assert.equal(first.expirunix, 1_767_348_960_000_000_000n)
    assert.deepEqual([first.crosscode, first.identifiers.quotereqid], ['Q1', 'R1'])
    // The update follows under the quote's identifier, and the cancel ends the chain.
    assert.equal(update.prevuuid, first.curruuid)
    assert.equal(update.bidpx, '10.45')
    assert.deepEqual([cancel.seqnum, cancel.state], [2, '90CANCELED'])
    assert.ok(!cancel.isAlive && cancel.bid === null)
    // A one-sided quote quotes one lane, and is about no mid.
    assert.deepEqual(oneSided.bid, { px: '10.3', qty: '50' })
    assert.equal(oneSided.isTwoSided, false)
    assert.deepEqual([oneSided.mid, oneSided.spread], [null, null])
    assert.equal(oneSided.event().px, '10.3') // about the bid it states
    // No one message states a quote.
    assert.throws(() => fix.FixMsg.fromQuote(reader, first), /does not guess/)
    const field = market.Quote.field()
    assert.equal(field.fieldAt(16).name, 'bidpx')
    assert.equal(market.Quote.fromRow(field, first.intoRow()).curruuid, first.curruuid)
    ```

## Row schema

`QuoteData::field()` opens with the sixteen [event columns](../graph.md#columns), then the two lanes and the instrument.

| column | datatype | value |
| --- | --- | --- |
| `bidpx` | `decimal128(38, 18)` | nullable; what the quoter would pay, `BidPx(132)` |
| `bidqty` | `decimal128(38, 18)` | nullable; for how much, `BidSize(134)` |
| `bidcurrency` | `currency` | nullable; the lane's currency, else the quote's where the lane is priced |
| `bidunit` | `utf8` | nullable; the lane's unit, else the quote's |
| `askpx` | `decimal128(38, 18)` | nullable; what the quoter would be paid, `OfferPx(133)` |
| `askqty` | `decimal128(38, 18)` | nullable; for how much, `OfferSize(135)` |
| `askcurrency` | `currency` | nullable; as the bid's |
| `askunit` | `utf8` | nullable; as the bid's |
| `symbolticker` | `utf8` | nullable; the ticker the instrument is known by |
| `isincode`, `cusipcode`, `sedolcode`, `bloombergcode` | `isin`, `cusip`, `sedol`, `bloomberg` | nullable; the instrument under each identifier the market named it by |
| `cficode` | `cfi` | nullable; the instrument's classification |
| `miccode` | `mic` | nullable; the market it is quoted on |

The quote's own price, currency and unit are the lane its side states, which the traits fill; a column for them would restate a lane, and `mid`, `spread` and the two lane readings are what the lanes imply. A quote's table joins the [message table](../fix/capture.md#the-crates-own-columns) on `srcuuids` to `curruuid` and its own table on `prevuuid` and `parentuuids`.

## Edges

- A cancel states no lane: its row's lane cells are null, `bid()` and `ask()` answer nothing, and its state ends the chain, so a later quote under the same identifier starts afresh.
- A lane stated at zero is withdrawn rather than quoted at nothing, so `bid()` answers nothing where either half of it is zero - and the [book](iterator.md) takes that lane off its ladder.
- A quote's validity is `expirunix`, from `ValidUntilTime(62)`; the walk retires a quote past it, and so does the book iterator's sweep.
- A two-sided quote is about no one price: `px` is zero and the row states none, while a one-sided quote's `px` is its lane's.

## Commands

```bash
cargo test -p yggdryl --test market quote::
cargo test -p yggdryl --doc market::quote
cargo bench -p yggdryl --bench market -- "market/doors/quotes|market/rows/quote"
```
