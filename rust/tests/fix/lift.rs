//! The facet table, the two role-addressed accessors, and the anomalies.

use std::path::PathBuf;
use std::sync::Arc;

use yggdryl::holder::local::Folder;
use yggdryl::{FixAnomaly, FixCodec, FixId, FixRegistry, Scalar};

fn reader() -> FixCodec {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder is a local path");
    FixCodec::new(Arc::new(
        FixRegistry::from_handle(&folder).expect("the committed dictionary loads"),
    ))
}

/// The text one facet answers with, for the assertions that read a spelling.
fn lifted<'msg>(message: &'msg yggdryl::FixMsg, facet: &str) -> Option<&'msg str> {
    message.lifted(facet).and_then(Scalar::as_str)
}

#[test]
fn an_order_answers_who_what_how_much_and_when() {
    let reader = reader();
    let order = reader
        .transform_line(b"8=FIX.4.4|35=D|49=SENDER|56=TARGET|34=7|11=ORDER-1|55=AAPL|54=1|38=100|44=12.5|60=20240102-10:15:30.000|10=0|", false)
        .unwrap();

    assert_eq!(lifted(&order, "id"), Some("ORDER-1"));
    assert_eq!(lifted(&order, "symbol"), Some("AAPL"));
    assert_eq!(lifted(&order, "side"), Some("1"));
    assert_eq!(lifted(&order, "sender"), Some("SENDER"));
    assert_eq!(lifted(&order, "target"), Some("TARGET"));
    assert_eq!(order.lifted("quantity"), Some(&Scalar::from(100.0_f64)));
    assert_eq!(order.lifted("price"), Some(&Scalar::from(12.5_f64)));
    assert!(order.lifted("transacttime").is_some());

    // A facet nothing answers is absent rather than an error, and a facet the
    // table does not have is absent too.
    assert_eq!(order.lifted("execid"), None);
    assert_eq!(order.lifted("nosuchfacet"), None);

    // The source is the tag that actually answered.
    assert_eq!(order.lift_source("id"), Some(FixId::standard(11)));
    assert_eq!(order.lift_source("quantity"), Some(FixId::standard(38)));
    assert_eq!(order.lift_source("nosuchfacet"), None);
}

#[test]
fn the_message_type_decides_which_source_a_facet_takes() {
    let reader = reader();
    // A fill states both `LastPx(31)` and `Price(44)`; the traded price is the
    // one an execution report means by `price`.
    let fill = reader
        .transform_line(
            b"8=FIX.4.4|35=8|17=EXEC-1|37=ORD-9|31=12.75|44=12.5|32=50|38=100|10=0|",
            false,
        )
        .unwrap();
    assert_eq!(fill.lifted("price"), Some(&Scalar::from(12.75_f64)));
    assert_eq!(fill.lift_source("price"), Some(FixId::standard(31)));
    assert_eq!(fill.lifted("quantity"), Some(&Scalar::from(50.0_f64)));
    assert_eq!(fill.lifted("execid"), Some(&Scalar::from("EXEC-1")));
    assert_eq!(lifted(&fill, "id"), Some("ORD-9"), "a fill is named by 37");

    // The same two tags on an order mean the order's own price.
    let order = reader
        .transform_line(
            b"8=FIX.4.4|35=D|11=ORDER-1|31=12.75|44=12.5|38=100|10=0|",
            false,
        )
        .unwrap();
    assert_eq!(order.lifted("price"), Some(&Scalar::from(12.5_f64)));
    assert_eq!(order.lift_source("price"), Some(FixId::standard(44)));
}

#[test]
fn a_fallback_is_visible_through_the_source_it_resolved_from() {
    let reader = reader();
    // No `TransactTime(60)`: the ladder falls to `SendingTime(52)`, which is a
    // different fact, so the fall has to be readable.
    let message = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|52=20240102-10:15:30.000|10=0|", false)
        .unwrap();
    assert_eq!(
        message.lifted("transacttime"),
        message.lifted("sendingtime")
    );
    assert_eq!(
        message.lift_source("transacttime"),
        Some(FixId::standard(52))
    );

    let exact = reader
        .transform_line(
            b"8=FIX.4.4|35=D|11=A|60=20240102-09:00:00.000|52=20240102-10:15:30.000|10=0|",
            false,
        )
        .unwrap();
    assert_eq!(exact.lift_source("transacttime"), Some(FixId::standard(60)));
    assert_ne!(exact.lifted("transacttime"), exact.lifted("sendingtime"));
}

#[test]
fn a_superseded_source_is_tried_after_every_current_one() {
    let reader = reader();
    // `QuantityType(465)` is superseded by `QtyType(854)`. A venue predating
    // the change carries only the old one and still answers.
    let old = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|38=100|465=1|10=0|", false)
        .unwrap();
    assert_eq!(old.lift_source("quantitytype"), Some(FixId::standard(465)));

    // Carrying both, the current one wins whichever order they arrived in.
    let both = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|38=100|465=1|854=2|10=0|", false)
        .unwrap();
    assert_eq!(both.lift_source("quantitytype"), Some(FixId::standard(854)));
}

#[test]
fn two_candidate_occurrences_answer_nothing_rather_than_the_first() {
    let reader = reader();
    // A multi-leg order has no one symbol, and saying so is the honest column.
    let legs = reader
        .transform_line(
            b"8=FIX.4.4|35=D|11=A|Symbol[0]=AAPL|Symbol[1]=MSFT|10=0|",
            false,
        )
        .unwrap();
    assert_eq!(legs.lifted("symbol"), None);
    assert_eq!(legs.lift_source("symbol"), None);
    // The occurrences are still in the row and still in the entries: only the
    // *column* declines to pick one.
    assert_eq!(
        legs.by_name("symbol")
            .unwrap()
            .as_sequence()
            .map(<[Scalar]>::len),
        Some(2)
    );
}

#[test]
fn a_quantity_is_answered_with_its_unit_whenever_the_message_states_one() {
    let reader = reader();
    let stated = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|53=1000000|854=5|15=USD|10=0|", false)
        .unwrap();
    let facets: Vec<&str> = stated.lift().map(|(facet, _)| facet).collect();
    assert!(facets.contains(&"quantity"), "{facets:?}");
    assert!(facets.contains(&"quantitytype"), "{facets:?}");
    // Where the type is currency, the number is money and `currency` says of
    // what. A reading, never a conversion.
    assert_eq!(lifted(&stated, "currency"), Some("USD"));

    // A number whose unit is unstated is answered as exactly that.
    let bare = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|53=1000000|10=0|", false)
        .unwrap();
    assert!(bare.lifted("quantity").is_some());
    assert_eq!(bare.lifted("quantitytype"), None);
}

#[test]
fn a_party_is_addressed_by_role_and_never_by_position() {
    let reader = reader();
    let row = "MSGTYPE=D|#NOPARTYIDS=2\
        |#NOPARTYIDS[0]=PARTYID=SYNTH-01\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1\
        |#NOPARTYIDS[1]=PARTYID=CLEARER-9\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=4";
    let message = reader.transform_line(row.as_bytes(), false).unwrap();

    // Asked for by name, matched through the code translation.
    let executing = message.party("ExecutingFirm").expect("the executing firm");
    assert_eq!(executing.id().and_then(Scalar::as_str), Some("SYNTH-01"));
    let clearing = message.party("ClearingFirm").expect("the clearing firm");
    assert_eq!(clearing.id().and_then(Scalar::as_str), Some("CLEARER-9"));
    assert_eq!(clearing.source().and_then(Scalar::as_str), Some("D"));
    assert!(clearing.role().is_some());
    assert_eq!(clearing.qualifier(), None);

    // A role nobody bears, and a role nobody has heard of, both answer None.
    assert_eq!(message.party("Custodian"), None);
    assert_eq!(message.party("NoSuchRole"), None);
    // `party` is not a facet: no occurrence is the message's.
    assert_eq!(message.lifted("party"), None);
}

#[test]
fn several_occurrences_bearing_one_role_answer_nothing() {
    let reader = reader();
    let row = "MSGTYPE=D|#NOPARTYIDS=2\
        |#NOPARTYIDS[0]=PARTYID=FIRST\x04\x03PARTYROLE=1\
        |#NOPARTYIDS[1]=PARTYID=SECOND\x04\x03PARTYROLE=1";
    let message = reader.transform_line(row.as_bytes(), false).unwrap();
    assert_eq!(message.party("ExecutingFirm"), None);
}

#[test]
fn a_regulatory_timestamp_is_addressed_by_its_type() {
    let reader = reader();
    let row = "MSGTYPE=8|#NOTRDREGTIMESTAMPS=1\
        |#NOTRDREGTIMESTAMPS[0]=TRDREGTIMESTAMP=20240102-10:15:30.000\x04\x03TRDREGTIMESTAMPTYPE=1";
    let message = reader.transform_line(row.as_bytes(), false).unwrap();
    assert!(message.trd_reg_timestamp("ExecutionTime").is_some());
    assert_eq!(message.trd_reg_timestamp("TimeIn"), None);
    assert_eq!(message.trd_reg_timestamp("NoSuchKind"), None);
}

#[test]
fn a_side_and_a_price_fill_their_lane_on_an_order_and_not_on_a_fill() {
    let reader = reader();
    // A buy order at `P` is a party willing to pay `P`, which is a bid.
    let buy = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|54=1|44=12.5|38=100|10=0|", false)
        .unwrap();
    assert_eq!(buy.lifted("bidpx"), Some(&Scalar::from(12.5_f64)));
    assert_eq!(buy.lifted("bidsize"), Some(&Scalar::from(100.0_f64)));
    assert_eq!(buy.lifted("askpx"), None);
    assert_eq!(buy.lifted("asksize"), None);
    // The derivation names the tag it came from, so it is never mistaken for
    // a quote the venue actually sent.
    assert_eq!(buy.lift_source("bidpx"), Some(FixId::standard(44)));

    let sell = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|54=2|44=12.5|38=100|10=0|", false)
        .unwrap();
    assert_eq!(sell.lifted("askpx"), Some(&Scalar::from(12.5_f64)));
    assert_eq!(sell.lifted("bidpx"), None);

    // A side taking no lane fills neither. `Cross` is both sides at once.
    let cross = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|54=8|44=12.5|38=100|10=0|", false)
        .unwrap();
    assert_eq!(cross.lifted("bidpx"), None);
    assert_eq!(cross.lifted("askpx"), None);

    // A fill's price is a traded price, not a quote lane: `LastPx` never
    // projects, and neither does a fill's `Price`.
    let fill = reader
        .transform_line(
            b"8=FIX.4.4|35=8|17=E|54=1|31=12.75|44=12.5|32=50|10=0|",
            false,
        )
        .unwrap();
    assert_eq!(fill.lifted("bidpx"), None);
    assert_eq!(fill.lifted("bidsize"), None);
}

#[test]
fn one_lane_implies_a_side_and_two_lanes_imply_nothing() {
    let reader = reader();
    let bidding = reader
        .transform_line(b"8=FIX.4.4|35=S|117=Q1|132=12.4|10=0|", false)
        .unwrap();
    assert_eq!(lifted(&bidding, "side"), Some("1"));
    assert_eq!(bidding.lift_source("side"), Some(FixId::standard(132)));

    let offering = reader
        .transform_line(b"8=FIX.4.4|35=S|117=Q1|133=12.6|10=0|", false)
        .unwrap();
    assert_eq!(lifted(&offering, "side"), Some("2"));

    // A two-sided quote is the case that makes the rule safe to have at all.
    let both = reader
        .transform_line(b"8=FIX.4.4|35=S|117=Q1|132=12.4|133=12.6|10=0|", false)
        .unwrap();
    assert_eq!(both.lifted("side"), None);

    // An order is not a quote, so no lane implies its side.
    let order = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|132=12.4|10=0|", false)
        .unwrap();
    assert_eq!(order.lifted("side"), None);
}

#[test]
fn enrichment_fills_and_never_overwrites() {
    let reader = reader();
    // A quote carrying one lane *and* a side answers the side as stated.
    let stated = reader
        .transform_line(b"8=FIX.4.4|35=S|117=Q1|132=12.4|54=2|10=0|", false)
        .unwrap();
    assert_eq!(lifted(&stated, "side"), Some("2"), "the stated side wins");
    assert_eq!(stated.lift_source("side"), Some(FixId::standard(54)));

    // An order stating its own bid lane keeps it rather than deriving one.
    let order = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|54=1|44=12.5|132=99.0|10=0|", false)
        .unwrap();
    assert_eq!(order.lifted("bidpx"), Some(&Scalar::from(99.0_f64)));
    assert_eq!(order.lift_source("bidpx"), Some(FixId::standard(132)));
}

#[test]
fn a_monitoring_facet_answers_on_a_row_nothing_else_could_type() {
    let reader = reader();
    // The row a monitor exists to see: no frame, no type, but a sequence
    // number and a session pair, which is what a capture is ordered by.
    let message = reader
        .transform_line(
            b"49=SENDER|56=TARGET|34=1092|43=Y|52=20240102-10:15:30.000",
            false,
        )
        .unwrap();
    assert_eq!(message.as_field().name(), "unknown");
    assert_eq!(message.lifted("seqnum"), Some(&Scalar::from(1092_i32)));
    assert_eq!(lifted(&message, "sender"), Some("SENDER"));
    assert_eq!(lifted(&message, "target"), Some("TARGET"));
    assert!(message.lifted("resent").is_some(), "a resend is not a gap");
    assert!(message.lifted("sendingtime").is_some());
}

#[test]
fn lift_yields_the_tables_own_order_and_stores_nothing() {
    let reader = reader();
    let message = reader
        .transform_line(
            b"8=FIX.4.4|35=D|34=7|11=ORDER-1|55=AAPL|54=1|38=100|10=0|",
            false,
        )
        .unwrap();

    let facets: Vec<&str> = message.lift().map(|(facet, _)| facet).collect();
    let ordered: Vec<&str> = yggdryl::fix_lifts()
        .map(yggdryl::FixLift::facet)
        .filter(|facet| facets.contains(facet))
        .collect();
    assert_eq!(facets, ordered, "the table's order, not the row's");

    // Two lifts are the same walk twice: nothing was cached and nothing in the
    // message changed.
    let again: Vec<&str> = message.lift().map(|(facet, _)| facet).collect();
    assert_eq!(facets, again);
    assert_eq!(message.into_text('|').unwrap().matches('=').count(), 8);

    // The table publishes what it reads, so a caller can project ahead of it.
    let tags: Vec<i32> = yggdryl::fix_lift("quantity").unwrap().tags().collect();
    assert!(tags.contains(&38) && tags.contains(&151), "{tags:?}");
}

#[test]
fn anomalies_are_derived_from_the_row_against_the_entries() {
    let reader = reader();
    // A `BodyLength` of `abc` will not type: the row holds null, the entry
    // holds the text, and the anomaly explains the null.
    let untyped = reader
        .transform_line(b"8=FIX.4.4|35=D|9=abc|11=A|10=0|", false)
        .unwrap();
    let found: Vec<FixAnomaly<'_>> = untyped.anomalies().collect();
    assert_eq!(
        found,
        [FixAnomaly::Untyped {
            tag: 9,
            key: "9",
            value: "abc",
        }],
        "{found:?}"
    );

    // A counter that counted an occurrence the row does not hold is stated,
    // never renumbered.
    let miscounted = reader
        .transform_line(b"MSGTYPE=D|#NOPARTYIDS=3|#NOPARTYIDS[0]=PARTYID=ONE", false)
        .unwrap();
    let found: Vec<FixAnomaly<'_>> = miscounted.anomalies().collect();
    assert_eq!(
        found,
        [FixAnomaly::Miscounted {
            tag: 453,
            name: "nopartyids",
            stated: 3,
            held: 1,
        }],
        "{found:?}"
    );
    assert_eq!(found[0].tag(), 453);
    assert!(found[0].to_string().contains("states 3 occurrences"));

    // A message that adds up reports nothing, and a caller who never asks
    // pays nothing either way.
    let clean = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|", false)
        .unwrap();
    assert_eq!(clean.anomalies().count(), 0);
}
