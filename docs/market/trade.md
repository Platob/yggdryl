# Trade

`market::Trade` is the settled transaction an execution reports: the matched quantity at its price, its identifier, its parties and its clocks - read out of every execution report and trade capture report that states a quantity that traded, the two sides' reports of one match in one chain.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `rust/src/market/trade.rs`: the `Trade` trait, the `TradeData` holder, `Party`, the row and the `Product` answers; the FIX reading is `FixCodec::trades` beside the [codec](../fix/index.md) |
| Chain | the match's: `crosscode` is `TrdMatchID(880)`, else `TradeID(1003)`, else `TradeReportID(571)`, else the report's `ExecID(17)`, and `crossuuid` the identity it derives; the names a trade goes by are the match's, the report's and the execution's - `tradereportid`, `tradeid`, `secondarytradeid`, `execid`, `secondaryexecid`, `trdmatchid` - and never the order's, which every trade of one order shares and which would chain them as one |
| Facts | `px` and `qty` the matched price and quantity; the side the reporting side's; the currency and unit; the ticker and the six codes; `tradedate` and `settldate`, `TradeDate(75)` and `SettlDate(64)` as days since the epoch; `parties`, FIX's `Parties` group as the venue stated it - the role, the identifier and its source of each occurrence, text because who names a party and how is the venue's to say |
| Reads | `notional()` what the match was worth; `party_by_role(role)` the first party in that role, matched as the report spells it; `settlement_days()` how many days after the trade date it settles, where both are stated |
| State | what the execution's own type says - `ExecType(150)`: traded, corrected, cancelled - never where the order it filled stands; a filled order would end the trade's chain before the other side's report reached it |
| Identity | the market's facts, the two dates and the parties digested with the event's own; a hop that added a party states a different trade, which chains under the same match |
| Doors | `FixCodec::trades(messages)`: one per execution report (`35=8`) or trade capture report (`AE`) stating a quantity that traded; `trades_arrow_reader(source)` under `TradeData::field()` |
| Writes back | refused: `FixMsg::from_trade` answers the one sentence - a trade is a reading of the executions that matched, and no one message states a match, so the crate does not guess |
| Bindings | Rust `Trade`/`TradeData`; Python `yggdryl.market.Trade`, `FixCodec.trades`, `trades_arrow_reader`, `FixMsg.from_trade`; JavaScript `market.Trade`, `trades`, `tradesArrowReader`, `FixMsg.fromTrade` |

## Use

The buy side's report of a match with its parties and clocks, and the sell side's report of the same match a moment later.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::local::Folder;
    use yggdryl::market::{Party, Product, Trade, TradeData};
    use yggdryl::{Decimal18, FixCodec, FixMsg, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let buy: &[u8] = b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|31=10.5|32=40|55=AAPL|54=1|15=USD|75=20260102|64=20260105|880=M1|453=2|448=FIRM|447=D|452=1|448=CLI-9|447=D|452=3|60=20260102-10:15:31.100|52=20260102-10:15:31.100|10=0|";
    let sell: &[u8] = b"8=FIX.4.4|35=8|11=B7|37=O2|17=E9|150=F|39=2|31=10.5|32=40|55=AAPL|54=2|15=USD|75=20260102|880=M1|60=20260102-10:15:31.200|52=20260102-10:15:31.200|10=0|";
    let trades: Vec<TradeData> = reader.trades(reader.parse_lines([buy, sell])).collect::<yggdryl::Result<_>>()?;
    let [bought, sold] = trades.as_slice() else { panic!("two trades") };
    assert_eq!((bought.get_px(), bought.get_qty()), ("10.5".parse()?, Decimal18::from_int(40)));
    assert_eq!(bought.get_side().as_str(), "BUY");
    assert_eq!(bought.get_tradedate(), Some(20_455), "2026-01-02 as days since the epoch");
    assert_eq!(bought.get_settldate(), Some(20_458));
    assert_eq!(
        bought.get_parties(),
        [
            Party { role: "1".to_owned(), id: "FIRM".to_owned(), source: "D".to_owned() },
            Party { role: "3".to_owned(), id: "CLI-9".to_owned(), source: "D".to_owned() },
        ]
    );
    // What the stated facts imply.
    assert_eq!(bought.notional(), Some(Decimal18::from_int(420)));
    assert_eq!(bought.party_by_role("3").map(|party| party.id.as_str()), Some("CLI-9"));
    assert_eq!(bought.party_by_role("9"), None, "a role is matched as the report spells it");
    assert_eq!(bought.settlement_days(), Some(3), "T+3");
    assert_eq!(sold.settlement_days(), None, "no settlement date stated");
    // The two sides share the match's chain, and the matched quantity counts once.
    assert_eq!(bought.get_crosscode(), "M1");
    assert_eq!(sold.get_crossuuid(), bought.get_crossuuid());
    assert_eq!(sold.get_prevuuid(), Some(bought.get_curruuid()));
    assert_eq!(bought.get_state().as_str(), "40TRADE", "the execution's type, not the order's status");
    let matched: Decimal18 = trades.iter().filter(|held| held.get_side().as_str() == "BUY").map(MarketElement::get_qty).sum();
    assert_eq!(matched, Decimal18::from_int(40));
    // No one message states a trade.
    let refused = FixMsg::from_trade(&reader, bought).unwrap_err().to_string();
    assert!(refused.contains("does not guess"), "{refused}");
    let field = TradeData::field()?;
    assert_eq!(field.fields().last().map(yggdryl::Field::name), Some("parties"));
    assert_eq!(TradeData::from_row(&field, &bought.into_row()?)?.get_curruuid(), bought.get_curruuid());
    ```

=== "Python"

    ```python
    import datetime
    from decimal import Decimal
    from pathlib import Path

    import pytest

    from yggdryl.fix import FixCodec, FixMsg, FixRegistry
    from yggdryl.market import Trade

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    buy = b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|31=10.5|32=40|55=AAPL|54=1|15=USD|75=20260102|64=20260105|880=M1|453=2|448=FIRM|447=D|452=1|448=CLI-9|447=D|452=3|60=20260102-10:15:31.100|52=20260102-10:15:31.100|10=0|"
    sell = b"8=FIX.4.4|35=8|11=B7|37=O2|17=E9|150=F|39=2|31=10.5|32=40|55=AAPL|54=2|15=USD|75=20260102|880=M1|60=20260102-10:15:31.200|52=20260102-10:15:31.200|10=0|"
    bought, sold = reader.trades(reader.parse_lines([buy, sell]))
    assert (bought.px.as_py(), bought.qty.as_py()) == (Decimal("10.5"), Decimal("40"))
    assert bought.side.as_py() == "BUY"
    assert bought.tradedate == datetime.date(2026, 1, 2)
    assert bought.settldate == datetime.date(2026, 1, 5)
    assert bought.parties == [("1", "FIRM", "D"), ("3", "CLI-9", "D")]
    # What the stated facts imply.
    assert bought.notional.as_py() == Decimal("420")
    assert bought.party_by_role("3") == ("3", "CLI-9", "D")
    assert bought.party_by_role("9") is None  # a role is matched as the report spells it
    assert bought.settlement_days == 3  # T+3
    assert sold.settlement_days is None  # no settlement date stated
    # The two sides share the match's chain, and the matched quantity counts once.
    assert bought.crosscode == "M1"
    assert sold.crossuuid == bought.crossuuid
    assert sold.prevuuid == bought.curruuid
    assert bought.state.as_py() == "40TRADE"  # the execution's type, not the order's status
    assert sum(held.qty.as_py() for held in (bought, sold) if held.side.as_py() == "BUY") == Decimal("40")
    # No one message states a trade.
    with pytest.raises(ValueError, match="does not guess"):
        FixMsg.from_trade(reader, bought)
    field = Trade.field()
    assert [child.name for child in field][-1] == "parties"
    assert Trade.from_row(field, bought.into_row()).curruuid == bought.curruuid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix, market } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)
    const buy = Buffer.from('8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|31=10.5|32=40|55=AAPL|54=1|15=USD|75=20260102|64=20260105|880=M1|453=2|448=FIRM|447=D|452=1|448=CLI-9|447=D|452=3|60=20260102-10:15:31.100|52=20260102-10:15:31.100|10=0|')
    const sell = Buffer.from('8=FIX.4.4|35=8|11=B7|37=O2|17=E9|150=F|39=2|31=10.5|32=40|55=AAPL|54=2|15=USD|75=20260102|880=M1|60=20260102-10:15:31.200|52=20260102-10:15:31.200|10=0|')
    const [bought, sold] = [...reader.trades(reader.parseLines([buy, sell]))]
    assert.deepEqual([bought.px, bought.qty], ['10.5', '40'])
    assert.equal(bought.side, 'BUY')
    assert.equal(bought.tradedate, 20455) // 2026-01-02 as days since the epoch
    assert.equal(bought.settldate, 20458)
    assert.deepEqual(bought.parties, [
      { role: '1', id: 'FIRM', source: 'D' },
      { role: '3', id: 'CLI-9', source: 'D' },
    ])
    // What the stated facts imply.
    assert.equal(bought.notional, '420')
    assert.deepEqual(bought.partyByRole('3'), { role: '3', id: 'CLI-9', source: 'D' })
    assert.equal(bought.partyByRole('9'), null) // a role is matched as the report spells it
    assert.equal(bought.settlementDays, 3) // T+3
    assert.equal(sold.settlementDays, null) // no settlement date stated
    // The two sides share the match's chain, and the matched quantity counts once.
    assert.equal(bought.crosscode, 'M1')
    assert.equal(sold.crossuuid, bought.crossuuid)
    assert.equal(sold.prevuuid, bought.curruuid)
    assert.equal(bought.state, '40TRADE') // the execution's type, not the order's status
    assert.equal([bought, sold].filter((held) => held.side === 'BUY').length, 1)
    // No one message states a trade.
    assert.throws(() => fix.FixMsg.fromTrade(reader, bought), /does not guess/)
    const field = market.Trade.field()
    assert.equal(field.fieldAt(30).name, 'parties') // the last column
    assert.equal(market.Trade.fromRow(field, bought.intoRow()).curruuid, bought.curruuid)
    ```

## Row schema

`TradeData::field()` opens with the sixteen [event columns](../graph.md#columns), then the trade's own.

| column | datatype | value |
| --- | --- | --- |
| `px` | `decimal128(38, 18)` | nullable; the matched price |
| `qty` | `decimal128(38, 18)` | nullable; the matched quantity |
| `side` | `side` | required; the reporting side, `UNKNOWN` where none is stated |
| `currency` | `currency` | required; `XXX` where none is stated |
| `unit` | `utf8` | nullable; the unit the quantity is counted in |
| `symbolticker` | `utf8` | nullable; the ticker the instrument is known by |
| `isincode`, `cusipcode`, `sedolcode`, `bloombergcode` | `isin`, `cusip`, `sedol`, `bloomberg` | nullable; the instrument under each identifier the market named it by |
| `cficode` | `cfi` | nullable; the instrument's classification |
| `miccode` | `mic` | nullable; the market it traded on |
| `tradedate` | `date32` | nullable; the day the trade was done |
| `settldate` | `date32` | nullable; the day it settles |
| `parties` | `list<party: struct<role: utf8, id: utf8, source: utf8?>>` | nullable; the parties in the order the report states them, empty where it states none |

`notional`, `party_by_role` and `settlement_days` are what those columns imply and have none of their own. A trade's table joins the [message table](../fix/capture.md#the-crates-own-columns) on `srcuuids` to `curruuid`, the [execution table](execution.md) on the same column - the same report makes both - and its own table on `prevuuid` and `parentuuids`.

## Edges

- Hop copies of one report that name the same parties fold into one trade naming every copy among its sources; a hop that added a party states a different trade, chained under the same match, so the trade table over a bridge behind several hops has more rows than the execution table.
- A trade capture report names no order, so its `crosscode` is the report's own `TradeReportID` or `TradeID`.
- The parties' roles are the wire's codes where the dictionary read them to one and the venue's words where it did not, and `party_by_role` matches whichever of the two the report spelled.
- A trade is not fed to the [book](iterator.md) beside the execution of the same fill: the FIX door feeds executions, because a stream carrying both would print one fill twice.

## Commands

```bash
cargo test -p yggdryl --test market trade::
cargo test -p yggdryl --doc market::trade
cargo bench -p yggdryl --bench market -- "market/doors/trades|market/rows/trade"
```
