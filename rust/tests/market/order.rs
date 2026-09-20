//! An order: one life read out of its placement and every report against
//! it, chained, folded where a message was logged twice, published as one
//! row, and stated back as a new order single.

use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::market::{Order, OrderData, Pricing, Product, read_arrow_reader};
use yggdryl::{Decimal18, FixMsg};

use super::{codec, message_reader, parsed, product_rows, rows_of};

/// One order's life: the placement under the client's identifier, the
/// acknowledgement under the venue's, a partial fill and the fill naming
/// the venue's alone.
const LIFE: [&[u8]; 4] = [
    b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|59=0|15=USD|52=20260102-10:15:30.250|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|52=20260102-10:15:30.500|10=0|",
    b"8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|6=10.5|55=AAPL|54=1|52=20260102-10:15:31.100|10=0|",
    b"8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.5|32=60|6=10.5|55=AAPL|54=1|52=20260102-10:15:33.100|10=0|",
];

fn decimal(text: &str) -> Decimal18 {
    text.parse().expect("a number")
}

#[test]
fn an_order_is_its_placement_and_every_report_against_it_chained() {
    let codec = codec();
    let messages = parsed(&codec, &LIFE);
    let orders: Vec<OrderData> = codec
        .orders(messages.clone())
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    assert_eq!(orders.len(), 4, "one statement per message about the order");

    // Every statement names the message it was read from, and nothing else.
    let chained: Vec<FixMsg> = codec
        .lifecycle(messages)
        .collect::<yggdryl::Result<_>>()
        .expect("the lifecycle");
    for (order, message) in orders.iter().zip(&chained) {
        assert_eq!(order.get_srcuuids(), [message.get_curruuid()]);
        assert_eq!(order.get_currunix(), message.get_currunix());
    }

    // The chain is the order's: the placement opened it under the client's
    // identifier, the venue's reports joined it by the names it went by,
    // and the code the chain shares is the one it opened under.
    let [placement, ack, partial, fill] = orders.as_slice() else {
        panic!("four statements")
    };
    assert_eq!(placement.get_crosscode(), "A1");
    assert!(orders.iter().all(|held| held.get_crosscode() == "A1"));
    assert!(
        orders
            .iter()
            .all(|held| held.get_crossuuid() == placement.get_crossuuid())
    );
    assert_eq!(
        orders.iter().map(Event::get_seqnum).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert_eq!(ack.get_prevuuid(), Some(placement.get_curruuid()));
    assert_eq!(partial.get_prevuuid(), Some(ack.get_curruuid()));
    assert_eq!(fill.get_prevuuid(), Some(partial.get_curruuid()));
    assert_eq!(
        fill.get_parentuuids(),
        [
            placement.get_curruuid(),
            ack.get_curruuid(),
            partial.get_curruuid()
        ]
    );
    // The names an order goes by accrue along the chain.
    assert_eq!(fill.get_identifiers()["clordid"], "A1");
    assert_eq!(fill.get_identifiers()["orderid"], "O1");
    assert!(
        !fill.get_identifiers().contains_key("execid"),
        "an execution's identifier is not a name of the order"
    );

    // The quantity ordered against what filled and what is left, the price
    // ladder, the side, the instrument and how long it stands.
    assert_eq!(placement.get_qty(), Decimal18::from_int(100));
    assert_eq!(placement.get_px(), decimal("10.5"));
    assert_eq!(placement.get_tif(), Some("0"));
    assert_eq!(placement.get_currency().as_str(), "USD");
    assert_eq!(placement.get_symbolticker(), Some("AAPL"));
    assert_eq!(placement.get_side().as_str(), "BUY");
    assert_eq!(partial.get_cumqty(), Some(Decimal18::from_int(40)));
    assert_eq!(partial.get_leavesqty(), Some(Decimal18::from_int(60)));
    assert_eq!(partial.get_avgpx(), Some(decimal("10.5")));
    assert_eq!(fill.get_leavesqty(), Some(Decimal18::from_int(0)));
    // The step before each statement rides along the chain, and the
    // order's terms ride forward where a report restates none: the fill
    // spells no limit, and is about the limit the order was placed at.
    assert_eq!(partial.get_prevqty(), Some(Decimal18::from_int(100)));
    assert_eq!(fill.get_px(), decimal("10.5"));
    assert_eq!(fill.get_tif(), Some("0"));
    // What the stated facts imply.
    assert_eq!(placement.pricing(), Some(Pricing::Limit));
    assert!(placement.is_resting());
    assert_eq!(partial.remaining(), Decimal18::from_int(60));
    assert_eq!(partial.filled(), Decimal18::from_int(40));
    assert_eq!(partial.filled_ratio(), Some(decimal("0.4")));
    assert!(partial.is_resting());
    assert!(!fill.is_resting(), "a filled order rests nothing");

    // The lifecycle folds forward: the state the chain reached, the
    // earliest creation, and a chain that ended.
    assert_eq!(
        placement.get_state().as_str(),
        "00UNKNOWN",
        "a placement states no status"
    );
    assert_eq!(ack.get_state().as_str(), "20NEW");
    assert_eq!(partial.get_state().as_str(), "40PARTFILL");
    assert_eq!(fill.get_state().as_str(), "80FILLED");
    assert_eq!(fill.get_creaunix(), placement.get_creaunix());
    assert!(
        !fill.get_state().is_live(),
        "a filled order ended its chain"
    );

    // Each identity is what the statement states and when.
    for order in &orders {
        assert_eq!(order.get_curruuid(), order.time_uuid().expect("an instant"));
    }
}

#[test]
fn a_report_logged_at_two_hops_is_one_statement_counted_once() {
    let codec = codec();
    let mut lines = LIFE.to_vec();
    // The acknowledgement, logged again on its way through a bridge.
    lines.insert(2, LIFE[1]);
    let messages = parsed(&codec, &lines);
    let chained: Vec<FixMsg> = codec
        .lifecycle(messages.clone())
        .collect::<yggdryl::Result<_>>()
        .expect("the lifecycle");
    assert_eq!(chained.len(), 5, "the lifecycle yields the twin restated");
    assert_eq!(
        chained[1].get_curruuid(),
        chained[2].get_curruuid(),
        "one identity however many hops logged it"
    );
    let orders: Vec<OrderData> = codec
        .orders(messages)
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    assert_eq!(
        orders.len(),
        4,
        "the twin folded into the statement it restates"
    );
    assert_eq!(
        orders.iter().map(Event::get_seqnum).collect::<Vec<_>>(),
        [0, 1, 2, 3],
        "the chain grew by nothing"
    );
    assert_eq!(orders[1].get_srcuuids(), [chained[1].get_curruuid()]);
    assert_eq!(orders[3].get_seqnum(), 3);
}

#[test]
fn the_row_opens_with_the_sixteen_and_round_trips_through_arrow() {
    let codec = codec();
    let field = OrderData::field().expect("the order row");
    let names: Vec<&str> = field.fields().iter().map(yggdryl::Field::name).collect();
    assert_eq!(
        &names[..16],
        &yggdryl::graph::EventColumn::ALL.map(|c| c.name())
    );
    assert_eq!(
        &names[16..],
        &[
            "px",
            "avgpx",
            "qty",
            "cumqty",
            "leavesqty",
            "side",
            "currency",
            "unit",
            "tif",
            "tradable",
            "symbolticker",
            "isincode",
            "cusipcode",
            "sedolcode",
            "bloombergcode",
            "cficode",
            "miccode",
            "stoppx",
            "ordtype",
        ]
    );
    assert_eq!(
        field.field("tif").expect("tif").dtype(),
        &yggdryl::DataType::TimeInForce
    );
    assert_eq!(field.field("px").expect("px").dtype(), &Decimal18::dtype());

    let orders: Vec<OrderData> = codec
        .orders(parsed(&codec, &LIFE))
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    // One row per order, the row round-tripping through the value: what a
    // row leaves out is what the walk stated beside it - the step before
    // - and everything it states comes back.
    for order in &orders {
        let row = order.into_row().expect("a row");
        let again = OrderData::from_row(&field, &row).expect("the row reads");
        assert_eq!(again.into_row().expect("a row"), row);
        assert_eq!(again.get_curruuid(), order.get_curruuid());
        assert_eq!(again.get_stoppx(), order.get_stoppx());
        assert_eq!(again.get_qty(), order.get_qty());
        let mut settled = again.clone();
        settled.finalize();
        assert_eq!(
            settled.get_curruuid(),
            order.get_curruuid(),
            "the identity survives"
        );
    }
    // And through Arrow batches: the rows written and read back are the
    // rows that were written.
    let reader = yggdryl::market::arrow_reader(&field, orders.clone().into_iter().map(Ok), 2, 0)
        .expect("a reader");
    let again: Vec<OrderData> = read_arrow_reader(reader)
        .expect("the reader opens")
        .collect::<yggdryl::Result<_>>()
        .expect("every row reads");
    assert_eq!(
        product_rows(again.into_iter().map(Ok)),
        product_rows(orders.into_iter().map(Ok))
    );
}

#[test]
fn the_arrow_door_agrees_with_the_message_door_row_for_row() {
    let codec = codec();
    let messages = parsed(&codec, &LIFE);
    let rows = rows_of(
        codec
            .orders_arrow_reader(message_reader(&codec, messages.clone()))
            .expect("the order rows open"),
    );
    assert_eq!(rows, product_rows(codec.orders(messages)));
    assert_eq!(rows.len(), 4);
}

#[test]
fn an_order_states_a_new_order_single_and_nothing_states_an_unnamed_one() {
    let codec = codec();
    let placement = codec
        .orders(parsed(&codec, &LIFE[..1]))
        .next()
        .expect("the placement")
        .expect("it reads");
    let message = FixMsg::from_order(&codec, &placement).expect("a new order single");
    assert_eq!(message.header().msgtype(), "D");
    assert_eq!(message.by_tag(11).unwrap().as_str(), Some("A1"));
    assert_eq!(
        message.by_tag(40).unwrap().as_str(),
        Some("2"),
        "a limit order"
    );
    assert_eq!(message.get_px(), placement.get_px());
    assert_eq!(message.get_qty(), placement.get_qty());
    assert_eq!(message.get_side(), placement.get_side());
    assert_eq!(
        message.get_currunix(),
        placement.get_currunix(),
        "dated by the order, not a clock"
    );
    assert_eq!(message.get_srcuuids(), [placement.get_curruuid()]);
    // The message the order states reads back as the order, stating the
    // type the order implied: a new order single states one.
    let source = message.get_curruuid();
    let again = codec
        .orders([message])
        .next()
        .expect("the placement again")
        .expect("it reads");
    assert_eq!(
        (again.get_qty(), again.get_px()),
        (placement.get_qty(), placement.get_px())
    );
    assert_eq!(again.get_ordtype(), Some("2"));
    assert_eq!(again.pricing(), placement.pricing());
    assert_eq!(again.get_srcuuids(), [source]);

    // A stop order spells its type, a stated type is spelled as stated,
    // and an order naming nothing is refused.
    let mut stop = placement.clone();
    stop.set_stoppx(Some(decimal("9")));
    stop.finalize();
    let message = FixMsg::from_order(&codec, &stop).expect("a stop limit");
    assert_eq!(message.by_tag(40).unwrap().as_str(), Some("4"));
    assert_eq!(
        message.by_tag(99).unwrap(),
        yggdryl::Scalar::from(decimal("9"))
    );
    assert_eq!(stop.pricing(), Some(Pricing::StopLimit));
    let mut pegged = placement.clone();
    pegged.set_ordtype(Some("P".to_owned()));
    pegged.finalize();
    let message = FixMsg::from_order(&codec, &pegged).expect("a pegged order");
    assert_eq!(message.by_tag(40).unwrap().as_str(), Some("P"));
    assert_eq!(
        pegged.pricing(),
        None,
        "a type outside the four prices some other way"
    );
    let again = codec
        .orders([message])
        .next()
        .expect("the order")
        .expect("it reads");
    assert_eq!(
        again.get_ordtype(),
        Some("P"),
        "the type is read back as stated"
    );
    let unnamed = OrderData::at(7);
    let refused = FixMsg::from_order(&codec, &unnamed)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("clordid"), "{refused}");
}
