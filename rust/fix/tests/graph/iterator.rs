//! `rust/market/src/graph/iterator.rs`: the one walk over market operation events:
//! each event chained to the live one under its cross identity - or, where
//! that is alive under nothing, under a name a live event goes by - the
//! alive set kept as the lifecycle moves, and the caller's word on the order
//! taken or the order made.

use std::hash::Hasher;

use yggdryl_market::IdKey;

use yggdryl::graph::{Element, Event};
use yggdryl::xxhash::Xxh3;
use yggdryl::{State, Uuid};
use yggdryl_market::graph::{EventIterator, ExecutionEvent, Operation, OrderEvent};
use yggdryl_market::{IdType, Identifier};

/// The XXH3-64 of one cross code, as the element derives its cross hash.
fn crosshash(crosscode: &str) -> u64 {
    let mut state = Xxh3::new();
    state.write(crosscode.as_bytes());
    state.as_u64()
}

/// Asserts `event` derives its cross identity from the cross code it
/// stores: the cross hash the code's XXH3-64, the cross element the UUIDv8
/// over that hash.
fn assert_derived(event: &impl Element, what: &str) {
    assert_eq!(
        event.get_crosshashcode(),
        crosshash(event.get_crosscode()),
        "{what}: the cross hash"
    );
    assert_eq!(
        event.get_crossuuid(),
        Uuid::from_v8(u128::from(event.get_crosshashcode())),
        "{what}: the cross element"
    );
}

/// One identifier of a plain holder: a value of `kind` from `fix`.
fn identifier(kind: &IdType, value: &str) -> Identifier {
    Identifier::new(IdKey::base(kind.clone()), value).unwrap()
}

/// One nanosecond count per millisecond: a derived identity opens with the
/// microsecond its instant falls in, and the instants below are spaced a
/// whole millisecond apart, so no two of them ever share one.
const MS: i64 = 1_000_000;
const CL_ORD_ID: IdType = IdType::ClOrdId;

/// An instant a derived identity holds: `ms` milliseconds after one
/// evening in November 2023, UTC.
fn at(ms: i64) -> i64 {
    1_700_000_000_000_000_000 + ms * MS
}

/// One event of the thing `order` identifies across its life, at `ms`.
fn incarnation(order: &str, ms: i64) -> OrderEvent {
    let mut event = OrderEvent::at(at(ms));
    event.set_crosscode(order.to_owned());
    event.finalize();
    event
}

/// One partial fill of `order` at `ms`: an execution, whatever state it
/// reached, because the leaf's kind says so.
fn executed(order: &str, ms: i64) -> ExecutionEvent {
    let mut event = ExecutionEvent::at(at(ms));
    event.set_crosscode(order.to_owned());
    event.set_state(State::read("PartiallyFilled").unwrap());
    event.finalize();
    event
}

/// A FIX message held as market data walks as itself: a twin of the live
/// message, and a statement logged again after its chain moved on at its
/// instant, restate the message they repeat through its own reading.
#[test]
fn a_market_data_walk_restates_a_fix_twin_and_a_fix_statement_logged_again() {
    crate::install::installed();
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    use yggdryl_market::MarketDataKind;
    use yggdryl_market::graph::MarketData;

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/fix");
    let folder = yggdryl::local::LocalFolder::new(root).expect("the local seed path");
    let codec = FixCodec::new(std::sync::Arc::new(
        FixRegistry::from_handle(&folder).expect("the committed dictionary loads"),
    ));
    // Walked as market data, read back as the messages the walk yielded.
    let walked = |lines: &[&str]| -> Vec<FixMsg> {
        let arrived: Vec<MarketData> = lines
            .iter()
            .flat_map(|line| codec.parse_line(line.as_bytes()).expect("a readable line"))
            .map(|message| MarketData::from(message.expect("every frame parses")))
            .collect();
        EventIterator::new(arrived, true)
            .filter(|held| held.marketdatakind() == MarketDataKind::Order)
            .map(|held| FixMsg::try_from(held).expect("a FIX message walks as itself"))
            .collect()
    };
    let order =
        "8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30.000|11=A1|55=AAPL|54=1|38=100|10=0|";
    let ack = |sequence: u64| {
        format!(
            "8=FIX.4.4|35=8|49=T|56=S|34={sequence}|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|54=1|55=AAPL|10=0|"
        )
    };
    let fill = "8=FIX.4.4|35=8|49=T|56=S|34=2|52=20260102-10:15:30.500|11=A1|37=O1|17=E1|150=F|39=1|14=10|151=90|31=10|32=10|54=1|55=AAPL|10=0|";

    // The acknowledgement delivered again under another sequence: a twin.
    let twin = walked(&[order, &ack(1), &ack(2)]);
    let [order_walked, first, again] = twin.as_slice() else {
        panic!("three order messages, not {}", twin.len())
    };
    assert_eq!(first.get_prevuuid(), Some(order_walked.get_uuid()));
    assert_eq!(
        (again.get_uuid(), again.get_prevuuid()),
        (first.get_uuid(), first.get_prevuuid()),
        "the twin is the acknowledgement again"
    );

    // Delivered again after the fill that moved its chain on at its instant.
    let passed = walked(&[order, &ack(1), fill, &ack(3)]);
    let [_, first, fill, again] = passed.as_slice() else {
        panic!("four order messages, not {}", passed.len())
    };
    assert_eq!(fill.get_prevuuid(), Some(first.get_uuid()));
    assert_eq!(
        (again.get_uuid(), again.get_prevuuid()),
        (first.get_uuid(), first.get_prevuuid()),
        "the acknowledgement again, not a step after the fill"
    );
}

/// The walk over the one value every boundary crosses as: a chain holds one
/// market data kind, so an execution under its order's cross code is a chain
/// of its own and never follows the order; its twin restates it and the
/// chain grows by nothing, and a value the walk does not chain - an undated
/// order - is yielded where it is read and changes nothing.
#[test]
fn a_market_data_walk_chains_within_one_kind_and_passes_the_rest_through() {
    crate::install::installed();
    use yggdryl_market::graph::{MarketData, Order};

    let fill = executed("O-500", 20);
    let arrived = vec![
        MarketData::from(incarnation("O-500", 10)),
        MarketData::from(Order::new()),
        MarketData::from(fill.clone()),
        MarketData::from(fill),
        MarketData::from(incarnation("O-500", 30)),
    ];
    let mut walk = EventIterator::new(arrived, true);
    let order = walk.next().expect("the order");
    let undated = walk.next().expect("the undated order, as it came");
    assert!(undated.as_order().is_some());
    let second = walk.next().expect("the fill");
    let leaf = second
        .as_execution_event()
        .expect("the fill keeps its kind");
    assert_eq!(
        (leaf.get_seqnum(), leaf.get_prevuuid()),
        (0, None),
        "the fill follows no order"
    );
    let twin = walk.next().expect("the fill, logged again");
    assert_eq!(twin, second, "the chain grows by nothing");
    let third = walk.next().expect("the order's next statement");
    assert_eq!(
        (
            third.as_order_event().unwrap().get_seqnum(),
            third.as_order_event().unwrap().get_prevuuid()
        ),
        (0, Some(order.get_uuid()))
    );
    assert_eq!(walk.alive().count(), 2, "the order's chain and the fill's");

    // Across leaves: a FIX message following a typed order of its kind, and
    // a typed order stating no side following a FIX message, each stand
    // under the chain's side, stored cross code and cross element, whatever
    // following the other leaf answered.
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    use yggdryl_market::Side;
    use yggdryl_market::graph::Market;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/fix");
    let folder = yggdryl::local::LocalFolder::new(root).expect("the local seed path");
    let codec = FixCodec::new(std::sync::Arc::new(
        FixRegistry::from_handle(&folder).expect("the committed dictionary loads"),
    ));
    let message = |line: &str| -> MarketData {
        let message: FixMsg = codec
            .parse_line(line.as_bytes())
            .expect("a readable line")
            .next()
            .expect("one frame")
            .expect("the frame parses");
        MarketData::from(message)
    };
    let typed = sided("T-1", 10, Side::Buy, State::New, &[(CL_ORD_ID, "A1")]);
    let fix = message(
        "8=FIX.4.4|35=D|49=S|56=T|34=1|52=20231114-22:13:20.020|11=A1|55=AAPL|54=1|38=100|10=0|",
    );
    assert_eq!(fix.get_crosscode(), "10:1:A1");
    let walked: Vec<MarketData> =
        EventIterator::new(vec![MarketData::from(typed), fix], true).collect();
    assert!(
        walked[1].as_message::<FixMsg>().is_some(),
        "the message keeps its leaf"
    );
    assert_eq!(walked[1].get_side(), Side::Buy);
    assert_eq!(
        walked[1].get_crosscode(),
        "10:1:T-1",
        "the typed chain's code"
    );
    assert_derived(&walked[1], "a FIX follower of a typed order");
    assert_eq!(walked[1].get_crossuuid(), walked[0].get_crossuuid());

    let fix = message(
        "8=FIX.4.4|35=D|49=S|56=T|34=1|52=20231114-22:13:20.010|11=B1|55=AAPL|54=1|38=100|10=0|",
    );
    let unsided = sided("Z-9", 20, Side::Unknown, State::New, &[(CL_ORD_ID, "B1")]);
    assert_eq!(unsided.get_crosscode(), "10:0:Z-9");
    let walked: Vec<MarketData> =
        EventIterator::new(vec![fix, MarketData::from(unsided)], true).collect();
    let follower = walked[1]
        .as_order_event()
        .expect("the order keeps its leaf");
    assert_eq!(follower.get_side(), Side::Buy, "the FIX chain's side");
    assert_eq!(follower.get_crosscode(), "10:1:B1", "the FIX chain's code");
    assert_derived(follower, "a typed follower of a FIX message");
    assert_eq!(follower.get_crossuuid(), walked[0].get_crossuuid());
    assert_eq!(
        follower.get_uuid(),
        follower.time_uuid().expect("an identity")
    );
}

/// An event of `order` taking `side` at `ms` in `state`, going by `names`.
fn sided(
    order: &str,
    ms: i64,
    side: yggdryl_market::Side,
    state: State,
    names: &[(IdType, &str)],
) -> OrderEvent {
    use yggdryl_market::graph::Market;
    let mut event = OrderEvent::at(at(ms));
    event.set_crosscode(order.to_owned());
    event.set_side(side, true);
    event.set_state(state);
    for (scheme, name) in names {
        event.insert_identifier(identifier(scheme, name)).unwrap();
    }
    event.finalize();
    event
}
