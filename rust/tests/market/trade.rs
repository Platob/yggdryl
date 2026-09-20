//! A trade: the settled transaction an execution reports, with its
//! parties and its clocks, the two sides' reports of one match in one
//! chain, a report logged twice counted once, and no message that states
//! it back.

use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::market::{Party, Product, Trade, TradeData};
use yggdryl::{Decimal18, FixMsg};

use super::{codec, message_reader, parsed, product_rows, rows_of};

/// The buy side's report of a match, with its parties and its clocks, the
/// sell side's report of the same match a moment later, and a trade
/// capture report of another match.
const REPORTS: [&[u8]; 3] = [
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|31=10.5|32=40|55=AAPL|54=1|15=USD|75=20260102|64=20260105|880=M1|453=2|448=FIRM|447=D|452=1|448=CLI-9|447=D|452=3|60=20260102-10:15:31.100|52=20260102-10:15:31.100|10=0|",
    b"8=FIX.4.4|35=8|11=B7|37=O2|17=E9|150=F|39=2|31=10.5|32=40|55=AAPL|54=2|15=USD|75=20260102|880=M1|60=20260102-10:15:31.200|52=20260102-10:15:31.200|10=0|",
    b"8=FIX.4.4|35=AE|571=T1|1003=TR1|31=11|32=5|55=AAPL|54=1|15=USD|75=20260102|60=20260102-10:15:40.000|52=20260102-10:15:40.000|10=0|",
];

#[test]
fn a_trade_is_the_matched_quantity_at_its_price_with_its_parties_and_clocks() {
    let codec = codec();
    let trades: Vec<TradeData> = codec
        .trades(parsed(&codec, &REPORTS))
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    assert_eq!(trades.len(), 3);
    let [buy, sell, captured] = trades.as_slice() else {
        panic!("three trades")
    };
    assert_eq!(buy.get_px(), "10.5".parse().expect("a price"));
    assert_eq!(buy.get_qty(), Decimal18::from_int(40));
    assert_eq!(buy.get_side().as_str(), "BUY");
    assert_eq!(sell.get_side().as_str(), "SELL");
    assert_eq!(
        buy.get_tradedate(),
        Some(20_455),
        "2026-01-02 as days since the epoch"
    );
    assert_eq!(buy.get_settldate(), Some(20_458));
    assert_eq!(sell.get_settldate(), None);
    assert_eq!(
        buy.get_parties(),
        [
            Party {
                role: "1".to_owned(),
                id: "FIRM".to_owned(),
                source: "D".to_owned()
            },
            Party {
                role: "3".to_owned(),
                id: "CLI-9".to_owned(),
                source: "D".to_owned()
            },
        ]
    );
    assert_eq!(
        buy.party_by_role("3").map(|party| party.id.as_str()),
        Some("CLI-9")
    );
    assert_eq!(buy.party_by_role("9"), None);
    assert_eq!(buy.settlement_days(), Some(3));
    assert_eq!(sell.settlement_days(), None);
    assert_eq!(buy.notional(), Some(Decimal18::from_int(420)));
    // The chain is the match's: the two sides' reports share it.
    assert_eq!(buy.get_crosscode(), "M1");
    assert_eq!(sell.get_crossuuid(), buy.get_crossuuid());
    assert_eq!(sell.get_prevuuid(), Some(buy.get_curruuid()));
    assert_eq!(buy.get_identifiers()["execid"], "E1");
    assert!(
        !buy.get_identifiers().contains_key("orderid"),
        "the order's names would chain every trade of the order as one"
    );
    // A trade capture report names its own chain.
    assert_eq!(captured.get_crosscode(), "TR1");
    assert_eq!(captured.get_identifiers()["tradereportid"], "T1");
    assert_eq!(captured.get_qty(), Decimal18::from_int(5));
    assert_eq!(captured.get_seqnum(), 0);
    // Each is dated by its report and names it.
    let chained: Vec<FixMsg> = codec
        .lifecycle(parsed(&codec, &REPORTS))
        .collect::<yggdryl::Result<_>>()
        .expect("the lifecycle");
    for (trade, message) in trades.iter().zip(&chained) {
        assert_eq!(trade.get_srcuuids(), [message.get_curruuid()]);
        assert_eq!(trade.get_currunix(), message.get_currunix());
    }
}

#[test]
fn a_report_logged_twice_is_one_trade_and_its_quantity_counts_once() {
    let codec = codec();
    let lines: [&[u8]; 3] = [REPORTS[0], REPORTS[0], REPORTS[1]];
    let trades: Vec<TradeData> = codec
        .trades(parsed(&codec, &lines))
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    assert_eq!(trades.len(), 2, "the twin folded");
    let matched: Decimal18 = trades
        .iter()
        .filter(|held| held.get_side().as_str() == "BUY")
        .map(MarketElement::get_qty)
        .sum();
    assert_eq!(matched, Decimal18::from_int(40), "counted once");
    assert_eq!(trades[1].get_seqnum(), 1, "the chain grew by nothing");
}

#[test]
fn the_row_round_trips_and_the_arrow_door_agrees() {
    let codec = codec();
    let field = TradeData::field().expect("the trade row");
    assert_eq!(
        field.fields()[field.fields().len() - 3..]
            .iter()
            .map(yggdryl::Field::name)
            .collect::<Vec<_>>(),
        ["tradedate", "settldate", "parties"]
    );
    assert_eq!(
        field.field("tradedate").expect("tradedate").dtype(),
        &yggdryl::DataType::date32()
    );
    let messages = parsed(&codec, &REPORTS);
    let trades: Vec<TradeData> = codec
        .trades(messages.clone())
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    for trade in &trades {
        let row = trade.into_row().expect("a row");
        let again = TradeData::from_row(&field, &row).expect("reads");
        assert_eq!(again.into_row().expect("a row"), row);
        assert_eq!(again.get_curruuid(), trade.get_curruuid());
    }
    let rows = rows_of(
        codec
            .trades_arrow_reader(message_reader(&codec, messages.clone()))
            .expect("the trade rows open"),
    );
    assert_eq!(rows, product_rows(codec.trades(messages)));
    assert_eq!(rows.len(), 3);
}

#[test]
fn no_message_states_a_trade_and_the_refusal_says_why() {
    let codec = codec();
    let trade = codec
        .trades(parsed(&codec, &REPORTS))
        .next()
        .expect("a trade")
        .expect("it reads");
    let refused = FixMsg::from_trade(&codec, &trade).unwrap_err().to_string();
    assert!(
        refused.contains("trade") && refused.contains("does not guess"),
        "{refused}"
    );
}
