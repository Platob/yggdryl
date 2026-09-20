//! An execution: one fill, read by the narrow door out of the one report
//! that states it, in the chain of the order it fills, and stated back as
//! an execution report.

use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::market::{Execution, ExecutionData, Product};
use yggdryl::{Decimal18, FixMsg};

use super::{codec, message_reader, parsed, product_rows, rows_of};

/// The order's acknowledgement, which fills nothing, and its two fills.
const REPORTS: [&[u8]; 3] = [
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|15=USD|52=20260102-10:15:30.500|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|30=XNAS|55=AAPL|54=1|15=USD|880=M1|52=20260102-10:15:31.100|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.6|32=60|30=XNAS|55=AAPL|54=1|15=USD|880=M2|52=20260102-10:15:33.100|10=0|",
];

#[test]
fn only_a_report_stating_a_traded_quantity_is_an_execution() {
    let codec = codec();
    let fills: Vec<ExecutionData> = codec
        .executions(parsed(&codec, &REPORTS))
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    assert_eq!(fills.len(), 2, "the acknowledgement fills nothing");
    let [first, second] = fills.as_slice() else {
        panic!("two fills")
    };
    assert_eq!(first.get_px(), "10.5".parse().expect("a price"));
    assert_eq!(first.get_qty(), Decimal18::from_int(40));
    assert_eq!(second.get_px(), "10.6".parse().expect("a price"));
    assert_eq!(second.get_qty(), Decimal18::from_int(60));
    assert_eq!(first.get_side().as_str(), "BUY");
    assert_eq!(first.get_currency().as_str(), "USD");
    assert_eq!(first.get_miccode().map(yggdryl::Mic::as_str), Some("XNAS"));
    // The execution's own identifier is among its names; the order it fills
    // is its chain.
    assert_eq!(first.get_identifiers()["execid"], "E1");
    assert!(
        !first.get_identifiers().contains_key("trdmatchid"),
        "the match is the trade's name, which two orders' fills share"
    );
    assert_eq!(first.get_crosscode(), "O1");
    assert_eq!(second.get_crossuuid(), first.get_crossuuid());
    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(second.get_seqnum(), 1);
    assert!(
        first.is_partial() && !first.completes(),
        "the order stays open after it"
    );
    assert!(second.completes(), "the fill that completed the order");
    assert_eq!(first.notional(), Some(Decimal18::from_int(420)));
    // Each names the report it was read from.
    let chained: Vec<FixMsg> = codec
        .lifecycle(parsed(&codec, &REPORTS))
        .collect::<yggdryl::Result<_>>()
        .expect("the lifecycle");
    assert_eq!(first.get_srcuuids(), [chained[1].get_curruuid()]);
    assert_eq!(second.get_srcuuids(), [chained[2].get_curruuid()]);
}

#[test]
fn the_row_round_trips_and_the_arrow_door_agrees() {
    let codec = codec();
    let field = ExecutionData::field().expect("the execution row");
    assert_eq!(field.fields()[16].name(), "px");
    assert_eq!(
        field.fields().last().map(yggdryl::Field::name),
        Some("miccode")
    );
    let messages = parsed(&codec, &REPORTS);
    let fills: Vec<ExecutionData> = codec
        .executions(messages.clone())
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    for fill in &fills {
        let row = fill.into_row().expect("a row");
        let again = ExecutionData::from_row(&field, &row).expect("reads");
        assert_eq!(again.into_row().expect("a row"), row);
        assert_eq!(again.get_curruuid(), fill.get_curruuid());
    }
    let rows = rows_of(
        codec
            .executions_arrow_reader(message_reader(&codec, messages.clone()))
            .expect("the execution rows open"),
    );
    assert_eq!(rows, product_rows(codec.executions(messages)));
    assert_eq!(rows.len(), 2);
}

#[test]
fn an_execution_states_an_execution_report_and_an_unnamed_one_is_refused() {
    let codec = codec();
    let fill = codec
        .executions(parsed(&codec, &REPORTS))
        .next()
        .expect("the first fill")
        .expect("it reads");
    let message = FixMsg::from_execution(&codec, &fill).expect("an execution report");
    assert_eq!(message.header().msgtype(), "8");
    assert_eq!(message.by_tag(17).unwrap().as_str(), Some("E1"));
    assert_eq!(message.by_tag(37).unwrap().as_str(), Some("O1"));
    assert_eq!(message.by_tag(150).unwrap().as_str(), Some("F"));
    assert_eq!(
        message.by_tag(39).unwrap().as_str(),
        Some("1"),
        "still open"
    );
    assert_eq!(message.get_lastpx(), Some(fill.get_px()));
    assert_eq!(message.get_lastqty(), Some(fill.get_qty()));
    assert_eq!(message.get_miccode(), fill.get_miccode());
    assert_eq!(message.get_currunix(), fill.get_currunix());
    assert_eq!(message.get_srcuuids(), [fill.get_curruuid()]);
    // The report the fill states reads back as the fill.
    let again = codec
        .executions([message])
        .next()
        .expect("the fill again")
        .expect("it reads");
    assert_eq!(again.get_curruuid(), fill.get_curruuid());

    let refused = FixMsg::from_execution(&codec, &ExecutionData::at(7))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("execid"), "{refused}");
}
