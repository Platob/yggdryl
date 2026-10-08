//! `rust/src/fix/msg.rs`: the message holder's setters, and the row read back
//! into a message.

use super::SoleMessage;

use std::sync::Arc;

use yggdryl::fix::FIXENTRIES_COLUMN;

use yggdryl::graph::{Element, Event, Market, Operation};
use yggdryl::text::{TextBytes, TextLine};
use yggdryl::{
    DataType, Decimal, Field, FixCodec, FixEntry, FixMsg, FixRegistry, IdKey, IdSource, IdType,
    Identifier, Identifiers, Scalar, StructType, fix_schema, fix_schema_carrying,
};

/// One security identifier of `kind` a caller states, validated by its type
/// and from no named source.
fn securityid(kind: IdType, code: &str) -> Identifier {
    Identifier::new(IdKey::base(kind), code).expect("an identifier")
}

/// Every identifier of a set as `src:type=value`, in the set's order.
fn shown(ids: &Identifiers) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

/// What the wire states under `tag`, as it arrived.
fn wire_value(message: &FixMsg, tag: i32) -> Option<String> {
    message
        .entries()
        .iter()
        .find(|entry| entry.tag() == tag)
        .and_then(FixEntry::value)
        .map(str::to_owned)
}

fn reader() -> (Arc<FixRegistry>, FixCodec) {
    let registry = super::committed_registry();
    let reader = super::fixed_codec(Arc::clone(&registry));
    (registry, reader)
}

/// A status code set whose values are numbers - `QuoteStatus(297)`, typed
/// as an integer by the dictionary - states the message's state as a text
/// one does: a quote status report reads its quote as cancelled, expired or
/// rejected rather than unknown.
#[test]
fn an_integer_typed_status_tag_states_the_messages_state() {
    use yggdryl::State;

    let (_, reader) = reader();
    for (code, state) in [
        ("1", State::Canceled),
        ("7", State::Expired),
        ("5", State::Rejected),
    ] {
        let line = format!("8=FIX.4.4|35=AI|52=20260921-10:00:00|117=Q1|55=AAPL|297={code}|10=0|");
        let message = reader.sole_line(line.as_bytes()).expect("one message");
        assert_eq!(
            message.get_by_tag(297).and_then(|held| held.as_i64()),
            code.parse().ok(),
            "{code} is typed as an integer"
        );
        assert_eq!(*message.get_state(), state, "{code}");
    }
}

/// A setter states a security identifier as the message's fact and leaves
/// the wire as the source sent it: the `SecAltIDGrp(454)` occurrence the
/// message carried is still its one, and the identifier it read stands
/// beside the stated ones under its own source.
#[test]
fn instrument_identifier_setters_state_securityids_and_leave_the_wire() {
    let (_, reader) = reader();
    let mut message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A1|454=1|455=AAPL.O|456=5|10=0|")
        .expect("an order with one unrelated alternate identifier");
    assert_eq!(shown(message.get_securityids()), ["ric=AAPL.O"]);

    for (kind, code) in [
        (IdType::Isin, "US0378331005"),
        (IdType::Cusip, "037833100"),
        (IdType::Sedol, "2046251"),
        (IdType::Bloomberg, "AAPL US EQUITY"),
        (IdType::Figi, "BBG000BLNQ16"),
    ] {
        assert!(
            message
                .insert_securityid(securityid(kind, code))
                .expect("an identifier the message states")
        );
    }
    assert_eq!(
        shown(message.get_securityids()),
        [
            "bloomberg=AAPL US EQUITY",
            "cusip=037833100",
            "figi=BBG000BLNQ16",
            "isin=US0378331005",
            "ric=AAPL.O",
            "sedol=2046251",
        ],
        "held in the order of their keys, as `src:type` spells them"
    );
    assert!(message.get_by_name("secaltidgrp").is_none());
    assert_eq!(
        wire_value(&message, 454).as_deref(),
        Some("1"),
        "the wire as sent"
    );
    assert_eq!(wire_value(&message, 48), None, "no identifier is written");

    // A type and source's identifier is stated once: inserting under a held
    // one fills nothing, and another value for it is a removal and an
    // insertion. Removing one removes only that one; the RIC the input
    // stated remains.
    assert!(
        !message
            .insert_securityid(securityid(IdType::Isin, "US5949181045"))
            .unwrap()
    );
    assert!(
        message
            .remove_securityid(&IdKey::base(IdType::Isin))
            .unwrap()
    );
    assert!(
        message
            .insert_securityid(securityid(IdType::Isin, "US5949181045"))
            .unwrap()
    );
    assert!(
        message
            .remove_securityid(&IdKey::base(IdType::Bloomberg))
            .unwrap()
    );
    assert!(
        !message
            .remove_securityid(&IdKey::base(IdType::Bloomberg))
            .unwrap()
    );
    assert_eq!(
        message.get_securityids().get(&IdType::Isin),
        Some("US5949181045")
    );
    assert_eq!(message.get_securityids().get(&IdType::Bloomberg), None);

    for kind in [IdType::Isin, IdType::Cusip, IdType::Sedol, IdType::Figi] {
        assert!(
            message
                .remove_securityid(&IdKey::base(kind.clone()))
                .unwrap()
        );
    }
    assert_eq!(shown(message.get_securityids()), ["ric=AAPL.O"]);
    assert_eq!(wire_value(&message, 454).as_deref(), Some("1"));
}

/// What the message only derives - the national number its ISIN carries -
/// answers after what is stated: a stated identifier takes the derived one
/// of its type back, removing the ISIN takes back only what hung on it, and
/// the wire stays as the source sent it throughout.
#[test]
fn a_stated_identifier_replaces_a_derived_one() {
    let (_, reader) = reader();
    let mut message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A1|48=US0378331005|22=4|10=0|")
        .expect("an order stating its ISIN");
    assert_eq!(
        shown(message.get_securityids()),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );

    assert!(
        message
            .insert_securityid(securityid(IdType::Cusip, "037833100"))
            .unwrap()
    );
    assert_eq!(
        message
            .get_securityids()
            .get_from(&IdKey::new(IdSource::Derived, IdType::Cusip)),
        None
    );
    assert_eq!(
        message
            .get_securityids()
            .get_from(&IdKey::base(IdType::Cusip)),
        Some("037833100")
    );
    assert!(
        !message
            .insert_securityid(securityid(IdType::Cusip, "594918104"))
            .unwrap()
    );

    let mut derived = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A1|48=US0378331005|22=4|10=0|")
        .expect("an order stating its ISIN");
    assert!(
        derived
            .remove_securityid(&IdKey::base(IdType::Isin))
            .unwrap()
    );
    assert_eq!(derived.get_securityids().get(&IdType::Cusip), None);
    assert_eq!(wire_value(&derived, 48).as_deref(), Some("US0378331005"));
    assert!(
        message
            .remove_securityid(&IdKey::base(IdType::Isin))
            .unwrap()
    );
    assert_eq!(
        message.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    assert_eq!(wire_value(&message, 48).as_deref(), Some("US0378331005"));
    assert_eq!(wire_value(&message, 22).as_deref(), Some("4"));
}

/// Removing the ISIN takes back what was derived under it, and the
/// currency pair the symbol names - which hangs on the symbol - stands.
#[test]
fn removing_the_isin_keeps_the_pair_the_symbol_names() {
    let (_, reader) = reader();
    let mut message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A1|55=EUR/USD|22=4|48=US0378331005|54=1|38=1|10=0|")
        .expect("an order");
    assert_eq!(
        shown(message.get_securityids()),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "derived:forex=EUR/USD",
            "forex=EUR/USD",
            "isin=US0378331005"
        ]
    );
    assert!(
        message
            .remove_securityid(&IdKey::base(IdType::Isin))
            .unwrap()
    );
    assert_eq!(
        shown(message.get_securityids()),
        ["derived:forex=EUR/USD", "forex=EUR/USD"]
    );
    message.finalize();
    assert_eq!(
        shown(message.get_securityids()),
        ["derived:forex=EUR/USD", "forex=EUR/USD"],
        "and a settle restates neither"
    );
}

#[test]
fn crosscode_uses_fix_priority_while_session_events_name_the_observation() {
    let (registry, reader) = reader();
    let message = reader
        .sole_line(
            b"MSGTYPE=8|MSGSEQNUM=7|ORDERID=ORDER-1|CLORDID=CLIENT-1|ORIGCLORDID=CLIENT-0|QUOTEID=QUOTE-1|QUOTEREQID=REQUEST-1|MDREQID=MARKET-1|MSGSESSIONID=SESSION-1|MSGCTXID=CONTEXT-1",
        )
        .unwrap();

    // An execution report naming a `QuoteID(117)` is its quote's: the code
    // is stored under the quotation kind and no side.
    assert_eq!(message.get_crosscode(), "14:0:ORDER-1", "FIX priority wins");
    // The names the message goes by are its own; where the bridge delivered
    // it is the capture's word, so the session event is no identifier.
    assert_eq!(
        shown(message.get_identifiers()),
        [
            "clordid=CLIENT-1",
            "mdreqid=MARKET-1",
            "orderid=ORDER-1",
            "origclordid=CLIENT-0",
            "quoteid=QUOTE-1",
            "quotereqid=REQUEST-1",
        ],
        "every identifier a source field states stands, without the capture context"
    );
    // The client order identifier an order replaced is its parent: a
    // relation between the two types, which the registry answers.
    assert_eq!(
        registry.parents_of(&IdType::ClOrdId).as_ref(),
        [IdType::OrigClOrdId]
    );
    assert_eq!(
        registry.parent_of(&IdType::OrigClOrdId),
        Some((IdType::ClOrdId, 0))
    );
    assert_eq!(
        message.capture().msgsesseventid(),
        Some("8:SESSION-1:CONTEXT-1:7")
    );
    assert_eq!(
        message
            .by_tag(yggdryl::MSGSESSEVENTID_TAG_NAME.0)
            .unwrap()
            .as_str(),
        Some("8:SESSION-1:CONTEXT-1:7"),
        "answered by its tag like any capture fact"
    );

    let fallback = reader
        .sole_line(
            b"MSGTYPE=8|ORDERID=|CLORDID=|ORIGCLORDID=CLIENT-0|QUOTEID=QUOTE-1|QUOTEREQID=REQUEST-1|MDREQID=MARKET-1",
        )
        .unwrap();
    assert_eq!(
        fallback.get_crosscode(),
        "14:0:CLIENT-0",
        "first stated FIX id"
    );

    let capture_only = reader
        .sole_line(b"MSGTYPE=ZZ|MSGSEQNUM=7|MSGSESSIONID=SESSION-1|MSGCTXID=CONTEXT-1")
        .unwrap();
    assert_eq!(
        capture_only.get_crosscode(),
        "",
        "capture is not a chain id"
    );
    assert_eq!(
        capture_only.capture().msgsesseventid(),
        Some("ZZ:SESSION-1:CONTEXT-1:7")
    );
    assert!(capture_only.get_identifiers().is_empty());

    for partial in [
        b"MSGTYPE=ZZ|MSGSEQNUM=7|MSGSESSIONID=SESSION-1".as_slice(),
        b"MSGTYPE=ZZ|MSGSEQNUM=7|MSGCTXID=CONTEXT-1".as_slice(),
        b"MSGTYPE=ZZ|MSGSESSIONID=SESSION-1|MSGCTXID=CONTEXT-1".as_slice(),
    ] {
        let partial = reader.sole_line(partial).unwrap();
        assert_eq!(partial.capture().msgsesseventid(), None);
        assert!(partial.by_tag(yggdryl::MSGSESSEVENTID_TAG_NAME.0).is_err());
        assert!(
            !partial
                .get_identifiers()
                .contains_kind(&"msgsesseventid".parse::<IdType>().unwrap())
        );
        assert_eq!(partial.get_crosscode(), "");
    }

    let other_capture = reader
        .sole_line(
            b"MSGTYPE=8|MSGSEQNUM=7|ORDERID=ORDER-1|CLORDID=CLIENT-1|ORIGCLORDID=CLIENT-0|QUOTEID=QUOTE-1|QUOTEREQID=REQUEST-1|MDREQID=MARKET-1|MSGSESSIONID=SESSION-2|MSGCTXID=CONTEXT-2",
        )
        .unwrap();
    // Two deliveries of one message go by the same names and are one
    // content; only the session event they were delivered as tells them
    // apart.
    assert_eq!(message.get_identifiers(), other_capture.get_identifiers());
    assert_eq!(
        other_capture.capture().msgsesseventid(),
        Some("8:SESSION-2:CONTEXT-2:7")
    );
    assert_eq!(
        message.get_currhashcode(),
        other_capture.get_currhashcode(),
        "capture provenance is not message content"
    );
    assert_eq!(message.get_curruuid(), other_capture.get_curruuid());

    // The fixed row states it at its own column, and reads it back.
    let schema = fix_schema(&registry, "fix").unwrap();
    let row = message.into_row(&schema).unwrap();
    let at = schema
        .index_of(yggdryl::MSGSESSEVENTID_TAG_NAME.1)
        .expect("a msgsesseventid column");
    assert_eq!(
        row.as_sequence().expect("a row")[at].as_str(),
        Some("8:SESSION-1:CONTEXT-1:7")
    );
    let rebuilt = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(rebuilt.get_crosscode(), "14:0:ORDER-1");
    assert_eq!(rebuilt.get_identifiers(), message.get_identifiers());
    assert_eq!(
        rebuilt.capture().msgsesseventid(),
        Some("8:SESSION-1:CONTEXT-1:7")
    );
    assert_eq!(rebuilt.into_row(&schema).unwrap(), row);
}

/// A trade capture is keyed by the trade it reports - its `TradeID(1003)`,
/// else its `TradeReportID(571)` - ahead of any order tag it states, so two
/// reports of one order are two trades, and by its order's tags only where
/// it states neither. The trade tags lead for a trade alone: an execution
/// report naming the trade it filled stays its order's. A trade's lineage
/// fields are identifiers, each under the type its base names as a parent.
#[test]
fn a_trade_capture_is_keyed_by_its_trade_before_any_order_it_names() {
    let (registry, reader) = reader();
    let trade = |tags: &str| {
        reader
            .sole_line(format!("8=FIX.4.4|35=AE|{tags}|55=AAPL|32=10|31=100|10=0|").as_bytes())
            .expect("a trade capture")
    };
    assert_eq!(
        trade("571=TR-1|1003=T-1|11=C-1|37=O-1").get_crosscode(),
        "21:0:T-1",
        "the trade tags lead for a trade"
    );
    assert_eq!(trade("571=TR-1|11=C-1").get_crosscode(), "21:0:TR-1");
    assert_eq!(
        trade("11=C-1|37=O-1").get_crosscode(),
        "21:0:O-1",
        "a trade stating neither trade tag is keyed by its order's"
    );

    let replace = trade("571=TR-2|572=TR-1|487=2|1003=T-2|1126=T-1|1040=S-2|1127=S-1");
    assert_eq!(
        shown(replace.get_identifiers()),
        [
            "origsecondarytradeid=S-1",
            "origtradeid=T-1",
            "secondarytradeid=S-2",
            "tradeid=T-2",
            "tradereportid=TR-2",
            "tradereportrefid=TR-1",
        ]
    );
    assert_eq!(
        registry.parents_of(&IdType::TradeReportId).as_ref(),
        [IdType::TradeReportRefId],
        "the vocabulary's, which no field restates"
    );
    assert_eq!(
        registry.parent_of(&IdType::TradeReportRefId),
        Some((IdType::TradeReportId, 0))
    );
    let origtradeid: IdType = "origtradeid".parse().unwrap();
    assert_eq!(
        registry.parents_of(&IdType::TradeId).as_ref(),
        std::slice::from_ref(&origtradeid)
    );
    assert_eq!(registry.parent_of(&origtradeid), Some((IdType::TradeId, 0)));

    let report = reader
        .sole_line(b"8=FIX.4.4|35=8|37=O-1|11=C-1|1003=T-1|17=E-1|150=0|39=0|54=1|55=AAPL|10=0|")
        .unwrap();
    assert_eq!(report.get_crosscode(), "10:1:O-1", "an order's report");
}

/// A message's cross code is stored under the kind it is filed under and
/// the side it takes - `10:1:C1`, an order to buy - whatever prefix a code
/// is given with: another kind's or side's is replaced, the side a write
/// takes moves it, and a message of no side states side `0`.
#[test]
fn a_cross_code_is_stored_under_the_kind_and_the_side_the_message_takes() {
    let (_, reader) = reader();
    let mut order = reader
        .sole_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=1|40=2|10=0|")
        .unwrap();
    assert_eq!(order.get_crosscode(), "10:1:C1");
    order.set_crosscode("14:2:C9".to_owned());
    assert_eq!(
        order.get_crosscode(),
        "10:1:C9",
        "another kind and side replaced"
    );
    assert_eq!(order.get_crosshashcode(), yggdryl::xxhash::xxh3(b"10:1:C9"));
    assert_eq!(
        order.get_crossuuid(),
        yggdryl::Uuid::from_v8(u128::from(order.get_crosshashcode()))
    );
    order.set_crosscode("10:1:C9".to_owned());
    assert_eq!(
        order.get_crosscode(),
        "10:1:C9",
        "a code already stored is as given"
    );
    order.set(54, Scalar::from("2")).unwrap();
    assert_eq!(
        order.get_crosscode(),
        "10:2:C9",
        "the side a write takes moves it"
    );
    assert_eq!(order.get_crosshashcode(), yggdryl::xxhash::xxh3(b"10:2:C9"));

    // A message stating no side states `0`; so does any kind that is no
    // order or execution, whatever side its parts take - a quote's side is
    // a tag, which a write moves without moving its code.
    let none = reader
        .sole_line(b"8=FIX.4.4|35=D|11=C2|55=AAPL|38=1|40=2|10=0|")
        .unwrap();
    assert_eq!(none.get_crosscode(), "10:0:C2");
    let mut quote = reader
        .sole_line(b"8=FIX.4.4|35=S|117=Q5|55=AAPL|54=1|132=99|134=1|10=0|")
        .unwrap();
    assert_eq!(quote.get_crosscode(), "14:0:Q5");
    quote.set(54, Scalar::from("2")).unwrap();
    assert_eq!(quote.get_side().as_str(), "SELL");
    assert_eq!(quote.get_crosscode(), "14:0:Q5", "a tag moves no code");
    let trade = |line: &[u8]| {
        reader
            .parse_line(line)
            .unwrap()
            .next()
            .expect("the trade")
            .unwrap()
    };
    // The order tags a side states are its group's, not the trade's root.
    let unkeyed = trade(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|150=F|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=1|54=1|1427=E1|1009=10|37=O1|11=C3|10=0|",
    );
    assert_eq!(unkeyed.get_crosscode(), "", "an empty code stays empty");
    let mut trade = trade(
        b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=R1|150=F|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=1|54=1|1427=E1|1009=10|37=O1|11=C3|10=0|",
    );
    assert_eq!(
        trade.get_crosscode(),
        "21:0:R1",
        "a trade is keyed by its TradeReportID(571)"
    );
    trade.set_crosscode("T1".to_owned());
    assert_eq!(trade.get_crosscode(), "21:0:T1");
    trade.set_crosscode("10:1:T1".to_owned());
    assert_eq!(
        trade.get_crosscode(),
        "21:0:T1",
        "side 1 is an order's, not a trade's"
    );
}

#[test]
fn session_event_identifier_tracks_typed_capture_edits_without_losing_other_names() {
    let (_, reader) = reader();
    let mut message = reader
        .sole_line(
            b"MSGTYPE=8|MSGSEQNUM=7|ORDERID=ORDER-1|CLORDID=CLIENT-1|MSGSESSIONID=SESSION-1|MSGCTXID=CONTEXT-1",
        )
        .unwrap();

    message
        .set(yggdryl::MSGSESSIONID_TAG_NAME.0, Scalar::from("SESSION-2"))
        .unwrap();
    assert_eq!(
        message.capture().msgsesseventid(),
        Some("8:SESSION-2:CONTEXT-1:7")
    );
    assert_eq!(
        message.get_identifiers().get(&IdType::ClOrdId),
        Some("CLIENT-1")
    );
    assert_eq!(
        message.get_identifiers().get(&IdType::OrderId),
        Some("ORDER-1")
    );

    // A missing part unsays the key rather than leaving a stale one.
    message
        .set(yggdryl::MSGCTXID_TAG_NAME.0, Scalar::Null)
        .unwrap();
    assert_eq!(message.capture().msgsesseventid(), None);
    assert_eq!(
        message.get_identifiers().get(&IdType::ClOrdId),
        Some("CLIENT-1")
    );

    message
        .set(yggdryl::MSGCTXID_TAG_NAME.0, Scalar::from("CONTEXT-2"))
        .unwrap();
    assert_eq!(
        message.capture().msgsesseventid(),
        Some("8:SESSION-2:CONTEXT-2:7")
    );

    message.set(34, Scalar::from(8_u64)).unwrap();
    message.set(35, Scalar::from("F")).unwrap();
    assert_eq!(
        message.capture().msgsesseventid(),
        Some("F:SESSION-2:CONTEXT-2:8"),
        "header mutations resettle the complete delivery key"
    );

    let implicit_uuid = message.get_curruuid();
    message.set_crosscode("EXPLICIT".to_owned());
    // The code is stored under its kind and side - an order's, no side
    // stated - and the cross hash is the digest of the stored code.
    assert_eq!(message.get_crosscode(), "10:0:EXPLICIT");
    assert_eq!(
        message.get_crosshashcode(),
        yggdryl::xxhash::xxh3(b"10:0:EXPLICIT")
    );
    // Naming the chain moves both identities: the content code still leaves
    // the cross code out, while the event UUID seeds its payload with the
    // cross hash so two cross chains cannot project the same generic ID.
    assert_ne!(message.get_curruuid(), implicit_uuid);
    assert_eq!(message.get_curruuid(), message.time_uuid().unwrap());
    assert_eq!(
        message.get_crossuuid(),
        yggdryl::Uuid::from_v8(u128::from(message.get_crosshashcode()))
    );
    message
        .set(yggdryl::MSGSESSIONID_TAG_NAME.0, Scalar::from("SESSION-3"))
        .unwrap();
    assert_eq!(message.get_crosscode(), "10:0:EXPLICIT");
    assert_eq!(
        message.capture().msgsesseventid(),
        Some("F:SESSION-3:CONTEXT-2:8")
    );
    assert!(
        !message
            .get_identifiers()
            .contains_kind(&"msgsesseventid".parse::<IdType>().unwrap())
    );
}

/// The key is the four values joined by `:` and nothing else: no length
/// prefixes, each value exactly as stated. What that gives up is said here
/// too - a value holding a `:` of its own joins to the key of another
/// split - because a bridge names none of its sessions or contexts that way.
#[test]
fn session_event_identifier_is_the_plain_join_of_its_four_values() {
    let (registry, reader) = reader();
    let base = reader
        .sole_line(
            b"MSGTYPE=8|MSGSEQNUM=7|ORDERID=ORDER-1|MSGSESSIONID=SESSION-1|MSGCTXID=CONTEXT-1",
        )
        .unwrap();
    let with = |session: &str, context: &str| {
        let mut message = base.clone();
        message
            .set(yggdryl::MSGSESSIONID_TAG_NAME.0, Scalar::from(session))
            .unwrap();
        message
            .set(yggdryl::MSGCTXID_TAG_NAME.0, Scalar::from(context))
            .unwrap();
        message
    };

    // A `|` is an ordinary byte of a value: no part is length-prefixed.
    let left = with("A|B", "C");
    let right = with("A", "B|C");
    assert_eq!(left.capture().msgsesseventid(), Some("8:A|B:C:7"));
    assert_eq!(right.capture().msgsesseventid(), Some("8:A:B|C:7"));
    // The documented ambiguity: two splits of a `:` join to one key.
    let first = with("A:B", "C");
    let second = with("A", "B:C");
    assert_eq!(first.capture().msgsesseventid(), Some("8:A:B:C:7"));
    assert_eq!(
        first.capture().msgsesseventid(),
        second.capture().msgsesseventid()
    );
    // The sequence is its canonical decimal whatever spelled it.
    let padded = reader
        .sole_line(
            b"MSGTYPE=8|MSGSEQNUM=007|ORDERID=ORDER-1|MSGSESSIONID=SESSION-1|MSGCTXID=CONTEXT-1",
        )
        .unwrap();
    assert_eq!(
        padded.capture().msgsesseventid(),
        Some("8:SESSION-1:CONTEXT-1:7")
    );

    // A value stated for the key itself is never the message's word: it is
    // derived again from the four parts whenever the message settles.
    let mut stated = base.clone();
    stated
        .set(
            yggdryl::MSGSESSEVENTID_TAG_NAME.0,
            Scalar::from("1:8|9:SESSION-1|9:CONTEXT-1|7"),
        )
        .unwrap();
    assert_eq!(
        stated.capture().msgsesseventid(),
        Some("8:SESSION-1:CONTEXT-1:7")
    );

    // A row is read the same way: a stale cell is derived over, a null cell
    // is derived into, and the row written back states the one key.
    let schema = fix_schema(&registry, "fix").unwrap();
    let at = schema
        .index_of(yggdryl::MSGSESSEVENTID_TAG_NAME.1)
        .expect("a msgsesseventid column");
    let row = base.into_row(&schema).unwrap();
    for cell in [Scalar::from("8:SESSION-9:CONTEXT-1:7"), Scalar::Null] {
        let mut columns = row.as_sequence().expect("a row").to_vec();
        columns[at] = cell;
        let read = FixMsg::from_row(
            Arc::clone(&registry),
            &schema,
            &Scalar::from_sequence(columns),
        )
        .unwrap();
        assert_eq!(
            read.capture().msgsesseventid(),
            Some("8:SESSION-1:CONTEXT-1:7")
        );
        assert_eq!(read.into_row(&schema).unwrap(), row);
    }
    // And a row missing a part states no key, whatever its cell said.
    let context_at = schema
        .index_of(yggdryl::MSGCTXID_TAG_NAME.1)
        .expect("a msgctxid column");
    let mut columns = row.as_sequence().expect("a row").to_vec();
    columns[context_at] = Scalar::Null;
    let read = FixMsg::from_row(
        Arc::clone(&registry),
        &schema,
        &Scalar::from_sequence(columns),
    )
    .unwrap();
    assert_eq!(read.capture().msgsesseventid(), None);
    assert!(
        read.into_row(&schema)
            .unwrap()
            .as_sequence()
            .expect("a row")[at]
            .is_null(),
        "a partial delivery writes no key"
    );
}

/// A line's captures name the session and context a bridge delivered the
/// message over, its frame the type and the sequence: the four join, on the
/// capture and at the fixed row's own column, into the key two observations
/// of one delivery merge on - and into no identifier.
#[test]
fn a_captured_line_states_its_session_event_at_its_own_column() {
    let (registry, reader) = reader();
    let codec = reader.with_capture_names(["msgsessionid", "msgctxid"]);
    let options = Arc::new(yggdryl::text::TextOptions::new());
    let line = |body: &[u8], session: Option<&[u8]>, context: Option<&[u8]>| {
        TextLine::from_bytes(
            0,
            TextBytes::from_bytes(body).unwrap(),
            Arc::clone(&options),
        )
        .unwrap()
        .with_captures(vec![
            session.map(|held| TextBytes::from_bytes(held).unwrap()),
            context.map(|held| TextBytes::from_bytes(held).unwrap()),
        ])
        .unwrap()
        .with_handle_mtime(1_704_190_530_900_000_000)
    };
    let sole = |line: &TextLine| {
        codec
            .parse_text_line(line)
            .unwrap()
            .next()
            .expect("one message")
            .unwrap()
    };
    const WIRE: &[u8] = b"8=FIX.4.4|35=8|34=1094|37=O|11=C|10=0|";
    let captured = line(WIRE, Some(b"e7256476"), Some(b"9effef3e6a"));
    let message = sole(&captured);
    assert_eq!(message.capture().msgsessionid(), Some("e7256476"));
    assert_eq!(message.capture().msgctxid(), Some("9effef3e6a"));
    assert_eq!(
        message.capture().msgsesseventid(),
        Some("8:e7256476:9effef3e6a:1094")
    );
    assert_eq!(
        message
            .by_tag(yggdryl::MSGSESSEVENTID_TAG_NAME.0)
            .unwrap()
            .as_str(),
        Some("8:e7256476:9effef3e6a:1094")
    );
    assert!(
        !message
            .get_identifiers()
            .contains_kind(&"msgsesseventid".parse::<IdType>().unwrap())
    );
    // Delivery provenance, never content: the key is no byte of the wire.
    let wire = message.into_bytes(b'|');
    assert!(
        !String::from_utf8_lossy(&wire).contains("e7256476"),
        "{}",
        String::from_utf8_lossy(&wire)
    );

    // The row states it at its column, and a row read back holds it again.
    let schema = fix_schema(&registry, "fix").unwrap();
    let at = schema
        .index_of(yggdryl::MSGSESSEVENTID_TAG_NAME.1)
        .expect("a msgsesseventid column");
    let row = message.into_row(&schema).unwrap();
    assert_eq!(
        row.as_sequence().expect("a row")[at].as_str(),
        Some("8:e7256476:9effef3e6a:1094")
    );
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(
        held.capture().msgsesseventid(),
        message.capture().msgsesseventid()
    );
    assert_eq!(held.into_row(&schema).unwrap(), row);

    // The batch door states the same key at the same column, the captures
    // arriving as the batch's columns of those names - here a row header's.
    let headed = yggdryl::text::TextOptions::new()
        .try_with_rowheader(r"^(?P<msgsessionid>\w+) (?P<msgctxid>\w+) ")
        .unwrap();
    let headed_line = TextLine::from_bytes(
        0,
        TextBytes::from_bytes(b"e7256476 9effef3e6a 8=FIX.4.4|35=8|34=1094|37=O|11=C|10=0|")
            .unwrap(),
        Arc::new(headed.clone()),
    )
    .unwrap();
    assert_eq!(
        sole(&headed_line).capture().msgsesseventid(),
        Some("8:e7256476:9effef3e6a:1094"),
        "the line door reads a row header's captures alike"
    );
    let batch = yggdryl::text::into_arrow_batch(vec![headed_line], &headed).unwrap();
    let parsed = codec
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(
            batch.schema(),
            [batch.clone()],
        ))
        .unwrap()
        .map(std::result::Result::unwrap)
        .next()
        .expect("one batch");
    let rows =
        yggdryl::Serie::from_arrow_batch(None, &parsed, yggdryl::ArrowCastOptions::default())
            .expect("the rows");
    let cell = rows
        .scalar(0)
        .expect("the first row")
        .as_sequence()
        .expect("columns")[super::tag_index(&parsed, yggdryl::MSGSESSEVENTID_TAG_NAME.0)]
    .clone();
    assert_eq!(cell.as_str(), Some("8:e7256476:9effef3e6a:1094"));

    // A missing part states no key: no session, no context, or no sequence.
    for partial in [
        line(WIRE, None, Some(b"9effef3e6a")),
        line(WIRE, Some(b"e7256476"), None),
        line(
            b"8=FIX.4.4|35=8|37=O|11=C|10=0|",
            Some(b"e7256476"),
            Some(b"9effef3e6a"),
        ),
    ] {
        let partial = sole(&partial);
        assert_eq!(partial.capture().msgsesseventid(), None);
        assert!(
            !partial
                .get_identifiers()
                .contains_kind(&"msgsesseventid".parse::<IdType>().unwrap())
        );
        assert!(
            partial
                .into_row(&schema)
                .unwrap()
                .as_sequence()
                .expect("a row")[at]
                .is_null()
        );
    }
}

/// An execution report that states no execution clock executed when it
/// happened, and says so from the moment it is parsed: its `execunix` is its
/// `currunix`, rather than waiting for a lifecycle walk to date it. Only a
/// report of an execution is dated, and only a raw observation - a clock the
/// message states is its own, and a lifecycle output's state may be one it
/// inherited.
#[test]
fn a_parsed_execution_report_states_its_execution_at_its_instant() {
    const SENT: i64 = 1_704_190_530_100_000_000;
    const TRANSACTED: i64 = 1_704_190_530_050_000_000;
    const EXECUTED: i64 = 1_704_190_530_020_000_000;
    const FILL: &[u8] =
        b"8=FIX.4.4|35=8|52=20240102-10:15:30.100|37=O|17=E|150=F|39=2|14=100|151=0|10=0|";

    let (registry, reader) = reader();
    let fill = reader.sole_line(FILL).unwrap();
    assert!(fill.get_prevuuid().is_none(), "a raw observation");
    assert_eq!(fill.get_currunix(), SENT);
    assert_eq!(fill.get_execunix(), Some(SENT));

    // A request and an acknowledgement report no execution, so nothing
    // dates one.
    for line in [
        b"8=FIX.4.4|35=D|52=20240102-10:15:30.100|11=C|55=AAPL|54=1|38=100|10=0|".as_slice(),
        b"8=FIX.4.4|35=8|52=20240102-10:15:30.100|37=O|17=A|150=0|39=0|14=0|151=100|10=0|",
    ] {
        let held = reader.sole_line(line).unwrap();
        assert_eq!(held.get_currunix(), SENT);
        assert_eq!(
            held.get_execunix(),
            None,
            "{}",
            String::from_utf8_lossy(line)
        );
    }

    // A clock the report states is the one it executed at.
    let transacted = reader
        .sole_line(
            b"8=FIX.4.4|35=8|52=20240102-10:15:30.100|60=20240102-10:15:30.050|37=O|17=E|150=F|39=2|14=100|151=0|10=0|",
        )
        .unwrap();
    // TransactTime within the official delay of the sending clock is also
    // the report's instant.
    assert_eq!(transacted.get_currunix(), TRANSACTED);
    assert_eq!(transacted.get_execunix(), Some(TRANSACTED));
    let executed = reader
        .sole_line(
            b"8=FIX.4.4|35=8|52=20240102-10:15:30.100|2749=20240102-10:15:30.020|37=O|17=E|150=F|39=2|14=100|151=0|10=0|",
        )
        .unwrap();
    assert_eq!(executed.get_currunix(), SENT);
    assert_eq!(executed.get_execunix(), Some(EXECUTED));

    // The filled clock is a fact the row states, read back as stated, and
    // one a walk leaves where it is.
    let schema = fix_schema(&registry, "fix").unwrap();
    let at = schema
        .index_of(yggdryl::EXECUNIX_TAG_NAME.1)
        .expect("an execunix column");
    let row = fill.into_row(&schema).unwrap();
    assert_eq!(
        row.as_sequence().expect("a row")[at].temporal_count_at(yggdryl::TimeUnit::Nanosecond),
        Some(SENT)
    );
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(held.get_execunix(), Some(SENT));
    let walked = reader
        .lifecycle([fill.clone()])
        .next()
        .expect("one walked message")
        .unwrap();
    assert_eq!(walked.get_execunix(), Some(SENT));

    // A lifecycle output read back off a row stating no execution clock is
    // not dated again: its state may be what it followed, not what it did.
    let acked = reader
        .sole_line(
            b"8=FIX.4.4|35=8|52=20240102-10:15:30.000|37=O|17=A|150=0|39=0|14=0|151=100|10=0|",
        )
        .unwrap();
    let chained = reader
        .lifecycle([acked, fill])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(chained.len(), 2);
    let output = &chained[1];
    assert_eq!(output.get_prevuuid(), Some(chained[0].get_curruuid()));
    assert_eq!(output.get_execunix(), Some(SENT));
    let mut columns = output
        .into_row(&schema)
        .unwrap()
        .as_sequence()
        .expect("a row")
        .to_vec();
    columns[at] = Scalar::Null;
    let unstated = FixMsg::from_row(
        Arc::clone(&registry),
        &schema,
        &Scalar::from_sequence(columns),
    )
    .unwrap();
    assert!(unstated.get_prevuuid().is_some());
    assert_eq!(unstated.get_execunix(), None);
}

/// `ExpireDate(432)` is the last day an order can trade, so it stops being
/// good where that day ends - an order good until today is alive the day it
/// is placed, and its acknowledgement follows it - while `ExpireTime(126)` is
/// the instant it names. `MaturityDate(541)` is when the instrument matures,
/// no deadline of the message stating it.
#[test]
fn an_expire_date_is_good_through_its_day_and_a_maturity_is_no_deadline() {
    const PLACED: i64 = 1_789_984_800_000_000_000; // 2026-09-21T10:00:00Z
    const DAY_ENDS: i64 = 1_790_035_200_000_000_000; // 2026-09-22T00:00:00Z
    const AT_1630: i64 = 1_790_008_200_000_000_000; // 2026-09-21T16:30:00Z

    let (_, reader) = reader();
    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|38=10|44=100|40=2|59=6|432=20260921|10=0|",
        b"8=FIX.4.4|35=8|52=20260921-10:00:01|11=C1|37=O1|150=0|39=0|54=1|55=AAPL|59=6|432=20260921|10=0|",
    ];
    let parsed: Vec<FixMsg> = reader
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("two messages");
    let placed = &parsed[0];
    assert_eq!(placed.get_currunix(), PLACED);
    assert_eq!(placed.get_exprunix(), Some(DAY_ENDS));
    let chained: Vec<FixMsg> = reader
        .lifecycle(parsed)
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    let [order, ack, expired] = chained.as_slice() else {
        panic!(
            "the order, its acknowledgement and its expiry: {}",
            chained.len()
        )
    };
    assert_eq!(ack.get_prevuuid(), Some(order.get_curruuid()));
    assert_eq!(ack.get_crossuuid(), order.get_crossuuid());
    assert_eq!(expired.get_prevuuid(), Some(ack.get_curruuid()));
    assert_eq!(expired.get_currunix(), DAY_ENDS);
    assert_eq!(*expired.get_state(), yggdryl::State::Expired);

    // A written date reads back as the day it names, so the wire a walk
    // answers re-parses to the same deadline.
    let rewritten = reader
        .sole_line(order.into_text('|').unwrap().as_bytes())
        .unwrap();
    assert_eq!(rewritten.get_exprunix(), Some(DAY_ENDS));

    // An expiry time is exact, and wins over a date beside it.
    let timed = reader
        .sole_line(
            b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C2|55=AAPL|54=1|126=20260921-16:30:00|432=20260921|10=0|",
        )
        .unwrap();
    assert_eq!(timed.get_exprunix(), Some(AT_1630));

    // An instrument maturing today leaves the order without a deadline.
    let maturing = reader
        .sole_line(b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C3|55=ESZ6|54=1|541=20260921|10=0|")
        .unwrap();
    assert_eq!(maturing.get_exprunix(), None);
    let walked: Vec<FixMsg> = reader
        .lifecycle([maturing])
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    assert_eq!(walked.len(), 1, "nothing expires it");
}

const ORDER: &[u8] = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|VenueThing=7|9999=x|10=0|";

/// Every tag a message's children carry, beside the value each holds.
fn stated(message: &FixMsg) -> Vec<(i32, Scalar)> {
    message
        .as_field()
        .fields()
        .iter()
        .filter_map(|child| child.as_fix().tag().ok().flatten())
        .map(|tag| (tag, message.by_tag(tag).expect("an indexed tag")))
        .collect()
}

#[test]
fn a_set_value_is_typed_by_the_registry_field_and_appended_when_absent() {
    let (registry, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    let before = message.as_field().fields().len();
    let declared = registry.get_field_by_tag(1).expect("Account");

    message.set(1, Scalar::from("A-1")).unwrap();

    let fields = message.as_field().fields();
    assert_eq!(fields.len(), before + 1, "appended, not inserted");
    let child = fields.last().unwrap();
    assert_eq!(child.name(), declared.name(), "the dictionary's spelling");
    assert_eq!(child.dtype(), declared.dtype(), "the dictionary's type");
    assert_eq!(child.as_fix().tag().unwrap(), Some(1));
    assert!(!child.is_nullable(), "a stated value is non-null");
    assert_eq!(message.by_tag(1).unwrap().as_str(), Some("A-1"));
    assert_eq!(
        message.by_name("Account").unwrap().as_str(),
        Some("A-1"),
        "reached by name through the registry"
    );

    // A typed fact lands on its holder and never in the row.
    message.set(34, Scalar::from(7_i64)).unwrap();
    assert_eq!(message.as_field().fields().len(), before + 1);
    assert_eq!(message.header().msgseqnum(), Some(7));
    assert_eq!(message.by_tag(34).unwrap().as_u64(), Some(7));
}

#[test]
fn a_set_value_replaces_an_existing_child_in_place_and_keeps_the_tag_index() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    let before = stated(&message);
    let at = message
        .as_field()
        .index_of("symbol")
        .expect("the symbol child");

    message.set(55, Scalar::from("MSFT")).unwrap();
    message.set("Side", Scalar::from("2")).unwrap();

    assert_eq!(
        message.as_field().index_of("symbol"),
        Some(at),
        "same position"
    );
    assert_eq!(
        message.as_field().fields().len(),
        before.len() + 2,
        "two unknown children beside the tagged ones"
    );
    assert_eq!(message.by_tag(55).unwrap(), Scalar::from("MSFT"));
    assert_eq!(message.by_tag(54).unwrap().as_str(), Some("SELL"));
    // Content identity changes; every other tag still reaches its previous value.
    for (tag, value) in before {
        if tag == 55 || tag == 54 {
            continue;
        }
        if tag == yggdryl::CURRHASHCODE_TAG_NAME.0 {
            assert_ne!(message.by_tag(tag).unwrap(), value);
            continue;
        }
        assert_eq!(message.by_tag(tag).unwrap(), value, "tag {tag}");
    }
}

/// The entries are the row read as a tree, so a write is what they and the
/// wire re-emit: a replaced child spells its new value in place, an
/// appended one closes the wire, and a typed fact removed from its holder
/// leaves the wire.
#[test]
fn a_set_is_what_the_entries_and_the_wire_re_emit() {
    let (_, reader) = reader();
    let parsed = reader.sole_line(ORDER).unwrap();
    let mut message = parsed.clone();
    message.set(55, Scalar::from("MSFT")).unwrap();
    message.set(1, Scalar::from("A-1")).unwrap();
    assert_eq!(
        message
            .remove(54)
            .unwrap()
            .as_ref()
            .and_then(Scalar::as_str),
        Some("BUYS")
    );
    let symbol = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 55)
        .expect("the symbol entry");
    assert_eq!(symbol.name(), "symbol");
    assert_eq!(symbol.value(), Some("MSFT"));
    assert_eq!(
        message
            .entries()
            .last()
            .map(|entry| (entry.tag(), entry.value())),
        Some((1, Some("A-1")))
    );
    assert_eq!(
        message.into_bytes(b'|'),
        b"8=FIX.4.4|35=D|11=A1|55=MSFT|venuething=7|9999=x|59=0|1=A-1|10=0|"
    );
    assert_ne!(message.digest(), parsed.digest());
    assert_ne!(message.entries(), parsed.entries());
}

#[test]
fn a_boolean_reads_every_spelling_a_column_cast_reads() {
    // FIX spells a flag `Y` or `N`, and a bridge writes `yes` or `no`: each
    // is the generic value door's reading, as a column cast's, never a
    // refusal left null beside an anomaly.
    let (_, reader) = reader();
    for (spelling, flag) in [
        ("Y", true),
        ("yes", true),
        ("1", true),
        ("N", false),
        ("no", false),
        ("OFF", false),
    ] {
        let line = format!("8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|1028={spelling}|10=0|");
        let message = reader.sole_line(line.as_bytes()).expect("a message");
        assert_eq!(
            message.get_by_tag(1028),
            Some(Scalar::from(flag)),
            "{spelling:?}"
        );
        assert!(
            message.anomalies().is_empty(),
            "{spelling:?}: {:?}",
            message.anomalies()
        );
    }
    // Text no boolean spells stays a named refusal, the entry kept.
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|1028=maybe|10=0|")
        .expect("a message");
    assert_eq!(message.get_by_tag(1028), Some(Scalar::Null));
    assert_eq!(message.anomalies().len(), 1);
}

#[test]
fn a_status_a_dictionary_left_as_text_reads_as_the_number_it_spells() {
    // `SecurityTradingStatus(326)` and `TradSesStatus(340)` are numbers in the
    // committed dictionary and `SecurityStatus(965)` a code, spelled as text:
    // a dictionary that leaves all three text states the spelling the wire
    // carried, and the one integer reader reads each as the number it is.
    let fields = [
        ("securitytradingstatus", 326),
        ("tradsesstatus", 340),
        ("securitystatus", 965),
        ("symbol", 55),
    ]
    .map(|(name, tag)| {
        let mut field = DataType::utf8().nullable_field(name);
        field.as_fix_mut().set_tag(tag).expect("a tag");
        field
    });
    let registry = Arc::new(FixRegistry::from_fields(fields).expect("a dictionary"));
    let reader = super::fixed_codec(registry);
    for (body, expected) in [
        ("326=17|", Some(true)),
        ("326= 17 |", Some(true)),
        ("326=2|", Some(false)),
        ("326=7|", None),
        ("340=2|", Some(true)),
        ("340=3|", Some(false)),
        ("965=1|", Some(true)),
        ("965=3|", Some(true)),
        ("965=01|", Some(true)),
        ("965=5|", Some(false)),
        ("965=11|", Some(false)),
        ("965=7|", None),
        ("965=abc|", None),
        // The first status that answers decides.
        ("340=3|965=1|", Some(false)),
    ] {
        let line = format!("8=FIX.4.4|35=8|55=BRN|{body}10=0|");
        let held = reader.sole_line(line.as_bytes()).expect("a message");
        assert_eq!(held.get_tradable(), expected, "{line}");
    }
}

#[test]
fn a_null_is_stored_as_a_stated_null() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    message.set(55, Scalar::Null).unwrap();
    let at = message.as_field().index_of("symbol").unwrap();
    assert!(message.as_field().fields()[at].is_nullable());
    assert_eq!(message.get_by_tag(55), Some(Scalar::Null));
}

#[test]
fn an_unknown_name_is_refused_and_the_message_stands() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    let before = message.clone();
    let refused = message.set("nosuchfield", Scalar::from("y")).unwrap_err();
    assert!(refused.to_string().contains("nosuchfield"), "{refused}");
    assert_eq!(message, before);
    // A value the field refuses is refused the same way.
    assert!(message.set(38, Scalar::from("not a number")).is_err());
    assert_eq!(message, before);
}

#[test]
fn an_unknown_name_still_reaches_the_child_spelled_that_way() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    let at = message
        .as_field()
        .index_of("venuething")
        .expect("the venue's own child");
    message.set("Venue_Thing", Scalar::from("8")).unwrap();
    let child = &message.as_field().fields()[at];
    assert_eq!(child.name(), "venuething", "the child keeps its own field");
    assert_eq!(child.dtype(), &DataType::utf8());
    assert_eq!(message.by_name("venuething").unwrap(), Scalar::from("8"));
}

#[test]
fn a_bare_unknown_tag_is_appended_under_its_decimal_spelling() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    message.set(7777, Scalar::from("custom")).unwrap();
    let child = message.as_field().fields().last().unwrap();
    assert_eq!(child.name(), "7777");
    assert_eq!(child.dtype(), &DataType::utf8());
    assert!(child.is_nullable());
    assert_eq!(message.by_tag(7777).unwrap(), Scalar::from("custom"));
    // A second write reaches the same child rather than a second one.
    message.set(7777, Scalar::from("again")).unwrap();
    assert_eq!(message.by_tag(7777).unwrap(), Scalar::from("again"));
    assert_eq!(
        message.as_field().index_of("7777").map(|at| at + 1),
        Some(message.as_field().fields().len())
    );
    // The one the line already carried is replaced where it stands.
    message.set(9999, Scalar::from("y")).unwrap();
    assert_eq!(message.by_tag(9999).unwrap(), Scalar::from("y"));
}

#[test]
fn several_values_land_with_one_rebuild_as_the_same_writes_would_one_at_a_time() {
    let (_, reader) = reader();
    let mut many = reader.sole_line(ORDER).unwrap();
    let mut one = many.clone();
    let writes = [
        (55, Scalar::from("MSFT")),
        (1, Scalar::from("A-1")),
        (7777, Scalar::from("first")),
        (7777, Scalar::from("second")),
        (1, Scalar::from("A-2")),
    ];
    for (tag, value) in writes.clone() {
        one.set(tag, value).unwrap();
    }
    many.set_many(writes).unwrap();
    assert_eq!(many, one);
    assert_eq!(many.by_tag(1).unwrap().as_str(), Some("A-2"));
    assert_eq!(many.by_tag(7777).unwrap(), Scalar::from("second"));

    // One refused write refuses them all, and the message stands.
    let before = many.clone();
    assert!(
        many.set_many([(55, Scalar::from("X")), (38, Scalar::from("not a number"))])
            .is_err()
    );
    assert_eq!(many, before);
    many.set_many(Vec::<(i32, Scalar)>::new()).unwrap();
    assert_eq!(many, before);
}

#[test]
fn the_consuming_twin_answers_what_the_setter_leaves() {
    let (_, reader) = reader();
    let mut set = reader.sole_line(ORDER).unwrap();
    let with = set.clone().with_value(55, Scalar::from("MSFT")).unwrap();
    set.set(55, Scalar::from("MSFT")).unwrap();
    assert_eq!(with, set);
}

#[test]
fn remove_answers_the_value_and_the_other_tags_still_reach_their_children() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    let before = stated(&message);
    let count = message.as_field().fields().len();

    assert_eq!(message.remove(55).unwrap(), Some(Scalar::from("AAPL")));
    assert_eq!(message.get_by_tag(55), None);
    assert_eq!(message.as_field().fields().len(), count - 1);
    for (tag, value) in &before {
        if *tag == 55 {
            continue;
        }
        if *tag == yggdryl::CURRHASHCODE_TAG_NAME.0 {
            assert_ne!(message.by_tag(*tag).unwrap(), *value);
            continue;
        }
        assert_eq!(message.by_tag(*tag).unwrap(), *value, "tag {tag}");
    }
    // By name, by decimal, and a miss.
    assert_eq!(
        message.remove("VenueThing").unwrap(),
        Some(Scalar::from("7"))
    );
    assert_eq!(message.remove(9999).unwrap(), Some(Scalar::from("x")));
    assert_eq!(message.remove(55).unwrap(), None);
    assert_eq!(message.remove("nosuchfield").unwrap(), None);
    assert_eq!(message.as_field().fields().len(), count - 3);
    // The wire follows the row: what was removed is gone from it, and what
    // the dictionary derived for the order stays.
    assert_eq!(
        message.into_bytes(b'|'),
        b"8=FIX.4.4|35=D|11=A1|54=1|59=0|10=0|"
    );
}

#[test]
fn a_row_reads_back_into_the_message_that_made_it() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let mut parsed = reader.sole_line(ORDER).unwrap();
    parsed.set_recdunix(Some(100));
    parsed.set_execunix(Some(200), true);
    let row = parsed.into_row(&schema).unwrap();

    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();

    // The root a row reads back is the content row, not the fixed schema:
    // the typed facts are the holders' and never children of it.
    assert_eq!(held.as_field().name(), schema.name());
    // A fixed row is semantic: represented fields live in their columns,
    // so its reconstructed children need not retain wire arrival order.
    for tag in [8, 35, 11, 55, 54] {
        assert_eq!(
            held.by_tag(tag).unwrap(),
            parsed.by_tag(tag).unwrap(),
            "tag {tag}"
        );
    }
    // And it makes the row it came from, whole.
    assert_eq!(held.into_row(&schema).unwrap(), row);
    // Sending time was not stated by this line, so the row carries the
    // settled instant. The complete identity is stated in columns and must
    // survive rebuilding rather than being derived from a new entry order.
    assert!(!parsed.header().stated_sendingtime());
    assert!(!held.header().stated_sendingtime());
    assert_eq!(held.header().beginstring(), parsed.header().beginstring());
    assert_eq!(held.header().msgtype(), parsed.header().msgtype());
    assert_eq!(held.header().msgseqnum(), parsed.header().msgseqnum());
    assert_eq!(held.get_currunix(), parsed.get_currunix());
    assert_eq!(held.get_curruuid(), parsed.get_curruuid());
    assert_eq!(held.get_crossuuid(), parsed.get_crossuuid());
    assert_eq!(held.get_currhashcode(), parsed.get_currhashcode());
    assert_eq!(held.get_crosshashcode(), parsed.get_crosshashcode());
    assert_eq!(held.get_recdunix(), Some(100));
    assert_eq!(held.get_execunix(), Some(200));
}

/// The fixed row projecting the dictionary's `Parties(453)` group - its list
/// alone, its length the count - in place of the `partyids` identifiers:
/// the shape of a row that holds the group as a column, which the fixed row
/// itself keeps among its entries instead.
fn fixed_with_party_group(registry: &FixRegistry) -> Field {
    let fixed = fix_schema(registry, "fix").unwrap();
    // Every column of a row is nullable: a message states the group or not.
    let group = registry
        .field_by_counter(453)
        .expect("Parties")
        .clone()
        .with_nullable(true);
    let mut columns = Vec::new();
    for column in fixed.fields() {
        if column.name() == FIXENTRIES_COLUMN {
            columns.push(group.clone());
        }
        if column.name() != "partyids" {
            columns.push(column.clone());
        }
    }
    StructType::from_fields(columns)
        .map(DataType::from)
        .unwrap()
        .required_field("fix")
}

/// A repeating group is its list alone: `NoPartySubIDs(802)=0` is the list
/// holding nothing - an entry of its own, which re-emits - and a group never
/// stated is no list at all. A column holds a group as null or as at least
/// one occurrence, so the stated zero rides the residual record, and a table
/// storing a null list as an empty one - PyIceberg reads a null list of
/// structs back as `[]` - hands each message back as the parse wrote it:
/// settled again from its content, each keeps its content code and its
/// identity, and only the group stated empty re-emits.
#[test]
fn a_group_counting_none_is_stated_and_a_list_read_back_empty_is_not() {
    let (registry, reader) = reader();
    let party = b"8=FIX.4.4|35=D|49=S|56=T|34=7|11=A|55=AAPL|54=1|453=1|448=X|447=D|452=1|";
    let absent = reader.sole_line(&[&party[..], b"10=0|"].concat()).unwrap();
    let counted = reader
        .sole_line(&[&party[..], b"802=0|10=0|"].concat())
        .unwrap();
    assert_ne!(counted.entries(), absent.entries());
    assert_ne!(counted.get_currhashcode(), absent.get_currhashcode());
    assert_ne!(counted.digest(), absent.digest());
    let wire = |message: &FixMsg| String::from_utf8(message.into_bytes(b'|')).unwrap();
    assert!(
        wire(&counted).contains("|452=1|802=0|"),
        "{}",
        wire(&counted)
    );
    assert!(!wire(&absent).contains("802="), "{}", wire(&absent));

    // A row that does not record its content code is settled from what it
    // states, so the row is read without that column.
    let fixed = fixed_with_party_group(&registry);
    let hashcode_at = yggdryl::fix_column_of(&fixed, yggdryl::CURRHASHCODE_TAG_NAME.0)
        .expect("a currhashcode column");
    let schema = StructType::from_fields(
        fixed
            .fields()
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != hashcode_at)
            .map(|(_, column)| column.clone()),
    )
    .map(DataType::from)
    .unwrap()
    .required_field("fix");
    let parties_at = schema.index_of("parties").expect("a parties column");
    let DataType::Serie(party) = schema.fields()[parties_at].dtype() else {
        panic!("parties is a repeating group");
    };
    let subids_at = party.index_of("partysubids").expect("a partysubids member");
    assert!(
        party.index_of("nopartysubids").is_none(),
        "no counter member"
    );
    for (message, stated) in [(&absent, false), (&counted, true)] {
        let mut cells = message
            .into_row(&fixed)
            .unwrap()
            .as_sequence()
            .unwrap()
            .to_vec();
        cells.remove(hashcode_at);
        let written = Scalar::from_sequence(cells.clone());
        let mut members = cells[parties_at].as_sequence().expect("the parties")[0]
            .as_sequence()
            .expect("one party")
            .to_vec();
        assert!(members[subids_at].is_null(), "the subgroup's column");
        members[subids_at] = Scalar::from_sequence(Vec::<Scalar>::new());
        cells[parties_at] = Scalar::from_sequence([Scalar::from_sequence(members)]);
        let read_back = Scalar::from_sequence(cells);
        assert_ne!(read_back, written, "the list read back empty");
        let [as_written, as_read_back] = [written, read_back]
            .map(|row| FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap());
        for (held, read) in [(&as_written, "as written"), (&as_read_back, "as read back")] {
            assert_eq!(
                held.get_currhashcode(),
                message.get_currhashcode(),
                "{read}"
            );
            assert_eq!(held.get_curruuid(), message.get_curruuid(), "{read}");
            assert_eq!(wire(held).contains("|802=0|"), stated, "{read}");
        }
        // What the row states agrees, whichever way the list was stored.
        assert_eq!(as_read_back.entries(), as_written.entries());
        assert_eq!(as_read_back.digest(), as_written.digest());
        assert_eq!(wire(&as_read_back), wire(&as_written));
    }
}

/// The same rule at the root of a row: `NoPartyIDs(453)=0` is stated and
/// re-emits, and a `parties` group column read back as `[]` where the row held
/// null is the group absent, so a message settled again from the row keeps
/// its content code and its identity.
#[test]
fn a_root_group_counting_none_is_stated_and_a_list_read_back_empty_is_not() {
    let (registry, reader) = reader();
    let order = b"8=FIX.4.4|35=D|49=S|56=T|34=7|11=A|55=AAPL|54=1|";
    let absent = reader.sole_line(&[&order[..], b"10=0|"].concat()).unwrap();
    let counted = reader
        .sole_line(&[&order[..], b"453=0|10=0|"].concat())
        .unwrap();
    let wire = |message: &FixMsg| String::from_utf8(message.into_bytes(b'|')).unwrap();
    assert!(wire(&counted).contains("|453=0|"), "{}", wire(&counted));
    assert!(!wire(&absent).contains("453="), "{}", wire(&absent));

    // Read without the content code, so each message is settled from what
    // its row states.
    let fixed = fixed_with_party_group(&registry);
    let hashcode_at = yggdryl::fix_column_of(&fixed, yggdryl::CURRHASHCODE_TAG_NAME.0)
        .expect("a currhashcode column");
    let schema = StructType::from_fields(
        fixed
            .fields()
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != hashcode_at)
            .map(|(_, column)| column.clone()),
    )
    .map(DataType::from)
    .unwrap()
    .required_field("fix");
    let parties_at = schema.index_of("parties").expect("a parties column");
    for (message, stated) in [(&absent, false), (&counted, true)] {
        let written = message.into_row(&schema).unwrap();
        let mut cells = written.as_sequence().unwrap().to_vec();
        assert!(cells[parties_at].is_null(), "a column holds no empty group");
        cells[parties_at] = Scalar::from_sequence(Vec::<Scalar>::new());
        let read_back = Scalar::from_sequence(cells);
        for (row, read) in [(&written, "as written"), (&read_back, "as read back")] {
            let held = FixMsg::from_row(Arc::clone(&registry), &schema, row).unwrap();
            assert_eq!(
                held.get_currhashcode(),
                message.get_currhashcode(),
                "{read}"
            );
            assert_eq!(held.get_curruuid(), message.get_curruuid(), "{read}");
            assert_eq!(wire(&held).contains("|453="), stated, "{read}");
        }
    }
}

#[test]
fn a_data_field_that_is_not_text_is_held_as_the_decode_a_row_can_hold() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    // The one line no row can say what arrived on: `RawData(96)` carrying
    // bytes no text holds. Its length is stated, because that is what a data
    // field is for.
    let line: &[u8] = b"8=FIX.4.4\x0135=D\x0195=4\x0196=\xff\xfe A\x0110=000\x01";
    let parsed = reader.parse_fix_line(line).unwrap();

    // The row holds the bytes, typed as the data field is, and the entry
    // spells them as the text a column can read: the decode, so the wire
    // re-emits that rather than the bytes. That is the whole of what a row
    // cannot carry.
    assert_eq!(
        parsed.by_tag(96).unwrap(),
        Scalar::from(b"\xff\xfe A".to_vec())
    );
    let arrived = parsed
        .entries()
        .iter()
        .find(|entry| entry.tag() == 96)
        .expect("the data field");
    assert_eq!(arrived.value(), Some("\u{FFFD}\u{FFFD} A"));
    assert_ne!(parsed.into_bytes(1), line);

    // The residual record carries the decoded spelling, as it did on entry.
    let row = parsed.into_row(&schema).unwrap();
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(held.by_tag(95).unwrap(), parsed.by_tag(95).unwrap());
    assert_eq!(
        held.by_tag(96).unwrap(),
        Scalar::from("\u{FFFD}\u{FFFD} A".as_bytes().to_vec())
    );
    assert_eq!(held.into_row(&schema).unwrap(), row);
}

#[test]
fn the_same_line_read_as_text_is_the_decode_of_the_wire() {
    // A text line is text before the codec reads it. The bytes
    // that were not UTF-8 read as the Windows-1252 characters they are, so
    // the stated length - a count of wire bytes - reaches no boundary the
    // frame stated and is not honoured: the value stays what the frame cut,
    // and the message re-emits the line's text rather than the wire, beside
    // what the dictionary derived for the order. That the line was decoded
    // is the line's fact, and the line counts it.
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let wire: &[u8] = b"8=FIX.4.4\x0135=D\x0195=4\x0196=\xff\xfe A\x0110=000\x01";
    let line = TextLine::from_bytes(
        0,
        TextBytes::from_bytes(wire).unwrap(),
        std::sync::Arc::new(yggdryl::text::TextOptions::new()),
    )
    .unwrap();
    assert_eq!(line.decoded_byte_size(), 2);
    assert_eq!(
        line.body(),
        "8=FIX.4.4\u{1}35=D\u{1}95=4\u{1}96=\u{ff}\u{fe} A\u{1}10=000\u{1}"
    );

    let parsed = reader
        .parse_text_line(&line)
        .unwrap()
        .next()
        .expect("one message")
        .unwrap();
    let arrived = parsed
        .entries()
        .iter()
        .find(|entry| entry.tag() == 96)
        .expect("the data field");
    assert_eq!(arrived.value(), Some("\u{ff}\u{fe} A"));
    // `TimeInForce` is the dictionary's derivation for an order and an
    // ordinary child of the row, so it re-emits where the row carries it -
    // appended behind the content the line stated, in front of the trailer.
    assert_eq!(
        parsed.into_bytes(1),
        line.body()
            .replace("\u{1}10=", "\u{1}59=0\u{1}10=")
            .as_bytes()
    );
    assert_ne!(parsed.into_bytes(1), wire);

    // The semantic row preserves the typed data field and its length.
    let row = parsed.into_row(&schema).unwrap();
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(held.by_tag(95).unwrap(), parsed.by_tag(95).unwrap());
    assert_eq!(held.by_tag(96).unwrap(), parsed.by_tag(96).unwrap());
    assert_eq!(held.into_row(&schema).unwrap(), row);
}

#[test]
fn a_captures_own_columns_are_carried_by_the_message_and_never_its_content() {
    let (registry, reader) = reader();
    // Nullable, because a message states none of them - ever.
    let capture = StructType::from_fields([
        DataType::utf8().nullable_field("url"),
        DataType::Int64.nullable_field("rownum"),
        DataType::binary().nullable_field("body"),
        // The object the line came out of is one of them: no column of the
        // fixed row states it, so a capture that knows it declares it.
        DataType::url().nullable_field("sourceurl"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("line");
    let schema = fix_schema_carrying(&capture, &fix_schema(&registry, "fix").unwrap()).unwrap();
    let parsed = reader.sole_line(ORDER).unwrap();
    let at = |name: &str| schema.index_of(name).unwrap();

    // A parsed message has no capture columns: they are null in its row, and
    // a row read back off that one is the message it came from.
    let row = parsed.into_row(&schema).unwrap();
    for carrier in ["url", "rownum", "body", "sourceurl"] {
        assert!(row.get(at(carrier)).unwrap().is_null(), "{carrier}");
    }
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(held.by_tag(55).unwrap(), parsed.by_tag(55).unwrap());
    assert_eq!(held.into_row(&schema).unwrap(), row);

    // A row a reader put its own statements in - the one the crate tags
    // among them - says what the *line* was read from. Read back, the
    // message holds none of it: no child, no entry, nothing on the wire, and
    // nothing to answer by name or by tag.
    let mut columns = row.as_sequence().expect("a row").to_vec();
    columns[at("url")] = Scalar::from("file:///capture.log");
    columns[at("rownum")] = Scalar::from(42_i64);
    columns[at("body")] = Scalar::from(ORDER.to_vec());
    columns[at("sourceurl")] = Scalar::from(yggdryl::Url::from_str("file:///capture.log").unwrap());
    let carried = Scalar::from_sequence(columns);
    let again = FixMsg::from_row(Arc::clone(&registry), &schema, &carried).unwrap();
    assert_eq!(again.entries(), held.entries());
    assert_eq!(again.into_bytes(b'|'), held.into_bytes(b'|'));
    assert_eq!(again.by_tag(55).unwrap().as_str(), Some("AAPL"));
    for carrier in ["url", "rownum", "body", "sourceurl"] {
        assert!(again.as_field().index_of(carrier).is_none(), "{carrier}");
        assert!(again.get_by_name(carrier).is_none(), "{carrier}");
        assert!(
            !again.entries().iter().any(|entry| entry.name() == carrier),
            "{carrier}"
        );
        let wire = String::from_utf8_lossy(&again.into_bytes(b'|')).into_owned();
        assert!(!wire.contains(&format!("{carrier}=")), "{wire}");
    }
    assert!(again.capture().msgpluginid().is_none());
    assert!(
        again.get_by_tag(yggdryl::SOURCEURL_TAG_NAME.0).is_none(),
        "the object a line came out of is not a fact of the message"
    );

    // The message carries them out of the row - provenance beside its
    // content - and states them again at their columns, so the row read
    // back and written is the row it was read from.
    let written = again.into_row(&schema).unwrap();
    for carrier in ["url", "rownum", "body", "sourceurl"] {
        assert!(
            again.carried().iter().any(|(name, _)| name == carrier),
            "{carrier} is carried"
        );
        assert_eq!(
            written.get(at(carrier)),
            carried.get(at(carrier)),
            "{carrier}"
        );
    }
    assert_eq!(written, carried, "the row written again is the row read");
    // Parsed from bytes, a message carries nothing and states every one of
    // them null.
    let bare = parsed.into_row(&schema).unwrap();
    assert!(parsed.carried().is_empty());
    for carrier in ["url", "rownum", "body", "sourceurl"] {
        assert!(bare.get(at(carrier)).unwrap().is_null(), "{carrier}");
    }
    assert_eq!(bare, row);
}

/// Writing one of the capture's own columns onto a message is refused.
///
/// Silence would leave a caller believing the message states where its line
/// came from, and a row child would put `sourceurl=` on the wire.
#[test]
fn writing_a_captures_own_column_onto_a_message_is_refused() {
    let (_registry, reader) = reader();
    let mut parsed = reader.sole_line(ORDER).unwrap();
    let (tag, name) = yggdryl::SOURCEURL_TAG_NAME;
    let refusal = parsed
        .set(tag, Scalar::from("file:///capture.log"))
        .unwrap_err()
        .to_string();
    assert!(refusal.contains(name), "{refusal}");
    assert!(refusal.contains("capture"), "{refusal}");
    // Removing it reaches nothing rather than refusing: there was never a
    // fact there to clear.
    assert_eq!(parsed.remove(tag).unwrap(), None);
    // Refused and unchanged: the row grew nothing and the wire is the line.
    assert_eq!(
        parsed.into_bytes(b'|'),
        reader.sole_line(ORDER).unwrap().into_bytes(b'|')
    );
}

#[test]
fn a_row_without_the_entries_group_rebuilds_its_projected_content() {
    let (registry, reader) = reader();
    let wide = fix_schema(&registry, "fix").unwrap();
    // The residual record is dropped; the columns are all that is left.
    let columns: Vec<Field> = wide
        .fields()
        .iter()
        .filter(|column| column.name() != FIXENTRIES_COLUMN)
        .cloned()
        .collect();
    let narrow = StructType::from_fields(columns)
        .map(DataType::from)
        .unwrap()
        .required_field("fix");
    let parsed = reader.sole_line(ORDER).unwrap();
    let row = parsed.into_row(&narrow).unwrap();

    let held = FixMsg::from_row(Arc::clone(&registry), &narrow, &row).unwrap();
    // Ordinary tags rebuild from their projected columns even without the
    // optional residual record and its counter.
    for tag in [11, 54, 55] {
        assert_eq!(
            held.by_tag(tag).unwrap(),
            parsed.by_tag(tag).unwrap(),
            "tag {tag}"
        );
    }
    let wire = String::from_utf8(held.into_bytes(b'|')).unwrap();
    assert!(wire.starts_with("8=FIX.4.4|35=D|"), "{wire}");
    for field in ["|11=A1|", "|54=1|", "|55=AAPL|"] {
        assert!(wire.contains(field), "{wire}");
    }
    let again = held.into_row(&narrow).unwrap();
    assert_eq!(again, row);
    let twice = FixMsg::from_row(Arc::clone(&registry), &narrow, &again).unwrap();
    assert_eq!(twice.into_row(&narrow).unwrap(), again);
}

#[test]
fn a_deep_residual_reads_back_whole() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    // A trade report's sides, each side's parties, each party's
    // sub-identifiers: four counters deep, every level of it one JSON value
    // under the sides' key.
    const DEEP: &[u8] =
        b"8=FIX.4.4|35=AE|571=T1|552=1|54=1|453=1|448=P1|452=1|802=1|523=S1|803=1|10=0|";
    let parsed = reader.sole_line(DEEP).unwrap();
    fn depth(entries: &[FixEntry]) -> usize {
        entries
            .iter()
            .map(|entry| 1 + depth(entry.entries()))
            .max()
            .unwrap_or(0)
    }
    assert!(
        depth(parsed.entries()) >= 4,
        "the fixture nests deep: {:?}",
        parsed.entries()
    );
    let row = parsed.into_row(&schema).unwrap();
    let record = row
        .get(schema.index_of(FIXENTRIES_COLUMN).unwrap())
        .unwrap();
    let sides = record
        .as_mapping()
        .expect("the residual map")
        .iter()
        .find(|(key, _)| key.as_str().is_some_and(|key| key.starts_with("552:")))
        .and_then(|(_, value)| value.as_str().map(str::to_owned))
        .expect("the sides under their tag:name");
    assert!(sides.starts_with("[{"), "{sides}");
    assert!(sides.contains(r#""523:partysubid":"S1""#), "{sides}");

    // The JSON decodes back into the message: every pair the four levels
    // state is stated again, whatever order the map listed them in.
    fn pairs(entries: &[FixEntry], out: &mut Vec<(i32, String)>) {
        for entry in entries {
            if entry.entries().is_empty() {
                if let Some(value) = entry.value() {
                    out.push((entry.tag(), value.to_owned()));
                }
            } else {
                pairs(entry.entries(), out);
            }
        }
    }
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    let (mut stated, mut rebuilt) = (Vec::new(), Vec::new());
    pairs(parsed.entries(), &mut stated);
    pairs(held.entries(), &mut rebuilt);
    stated.sort();
    rebuilt.sort();
    assert_eq!(rebuilt, stated);
    assert!(
        stated.contains(&(523, "S1".to_owned())),
        "the deepest level's own pair: {stated:?}"
    );
    // A group read back out of the map opens each occurrence on its
    // delimiter again, as the dictionary declares its members.
    assert!(
        held.into_text('|')
            .unwrap()
            .contains("|453=1|448=P1|452=1|802=1|523=S1|803=1|"),
        "{}",
        held.into_text('|').unwrap()
    );
    assert_eq!(held.into_row(&schema).unwrap(), row);
}

/// A group nested in an occurrence is one entry under its counter, as a
/// group at the root is: the counter beside it states nothing the entry
/// does not, so the wire carries each counter once.
#[test]
fn a_nested_group_re_emits_its_counter_once() {
    let (_, reader) = reader();
    const DEEP: &[u8] =
        b"8=FIX.4.4|35=AE|571=T1|552=1|54=1|453=1|448=P1|452=1|802=1|523=S1|803=1|10=0|";
    let parsed = reader.sole_line(DEEP).unwrap();
    let side = parsed
        .entries()
        .iter()
        .find(|entry| entry.tag() == 552)
        .expect("the sides")
        .entries()
        .first()
        .expect("one side");
    assert_eq!(
        side.entries()
            .iter()
            .filter(|member| member.tag() == 453)
            .count(),
        1,
        "{:?}",
        side.entries()
    );
    assert_eq!(parsed.into_bytes(b'|'), DEEP);
}

#[test]
fn a_residual_value_whose_json_does_not_decode_is_refused() {
    let (registry, _) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let mut values: Vec<Scalar> = schema
        .fields()
        .iter()
        .map(|column| column.default_value().unwrap())
        .collect();
    values[schema.index_of(FIXENTRIES_COLUMN).unwrap()] =
        Scalar::from_mapping([(Scalar::from("58:text"), Scalar::from("{not json"))]).unwrap();
    let row = Scalar::from_sequence(values);
    let refused = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap_err();
    assert!(refused.to_string().contains("58:text"), "{refused}");
}

#[test]
fn a_malformed_residual_value_is_refused_instead_of_dropped() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let message = reader.sole_line(ORDER).unwrap();
    let original = message.into_row(&schema).unwrap();
    let at = schema.index_of(FIXENTRIES_COLUMN).unwrap();
    let row = |value: &str| {
        let mut values = original.as_sequence().unwrap().to_vec();
        values[at] =
            Scalar::from_mapping([(Scalar::from("453:parties"), Scalar::from(value))]).unwrap();
        Scalar::from_sequence(values)
    };
    for value in [
        // JSON that does not decode.
        "[1,",
        "{",
        r#""unterminated"#,
        // A member key that is no `tag:name`: a bare name, a bare tag, a
        // negative tag, a tag that is no number, an empty name.
        r#"[{"partyid":"P1"}]"#,
        r#"[{"448":"P1"}]"#,
        r#"[{"-1:partyid":"P1"}]"#,
        r#"[{"x:partyid":"P1"}]"#,
        r#"[{"448:":"P1"}]"#,
        // A member no text can state.
        r#"[{"448:partyid":{"1":"x"}}]"#,
    ] {
        let error = FixMsg::from_row(Arc::clone(&registry), &schema, &row(value)).unwrap_err();
        assert!(
            matches!(&error, yggdryl::Error::InvalidRecord { path, .. }
            if path.starts_with("$.fixentries")),
            "{value}: {error}"
        );
    }
    // What decodes is read rather than refused: an empty group, one
    // occurrence of a member the dictionary knows, a member it does not
    // know under tag zero, a number spelled as JSON spells one, and text
    // that opens no JSON at all.
    for value in [
        "[]",
        "{}",
        r#"[{"448:partyid":"P1"}]"#,
        r#"[{"0:venueseq":"7","448:partyid":"P1"}]"#,
        r#"[{"452:partyrole":1}]"#,
        "0",
        "",
    ] {
        assert!(
            FixMsg::from_row(Arc::clone(&registry), &schema, &row(value)).is_ok(),
            "{value}"
        );
    }
    assert!(
        FixMsg::from_row(Arc::clone(&registry), &schema, &row("[")).is_err(),
        "an array that never closes"
    );
    assert_eq!(message.into_row(&schema).unwrap(), original);
}

/// The execution intake dates survives the walk that places it: a successor
/// whose own execution is the later one carries it unchanged, so the walk
/// states nothing new and the settle behind it must not read the placed
/// message's fields over what the chain reached.
#[test]
fn a_walk_keeps_the_execution_intake_dated_on_a_successor() {
    const PARTIAL: i64 = 1_704_190_531_000_000_000;
    const FILLED: i64 = 1_704_190_532_000_000_000;
    let (_registry, reader) = reader();
    let partial = reader
        .sole_line(b"8=FIX.4.4|35=8|37=O1|150=F|39=1|52=20240102-10:15:31|10=0|")
        .unwrap();
    let filled = reader
        .sole_line(b"8=FIX.4.4|35=8|37=O1|150=F|39=2|52=20240102-10:15:32|10=0|")
        .unwrap();
    assert_eq!(partial.get_execunix(), Some(PARTIAL));
    assert_eq!(filled.get_execunix(), Some(FILLED));
    let walked = reader
        .lifecycle([partial, filled])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(walked.len(), 2);
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[0].get_execunix(), Some(PARTIAL));
    assert_eq!(walked[1].get_execunix(), Some(FILLED));
}

/// A group a row holds as a column - what a row read out of Arrow carries -
/// answers every reading the run the parse built answers: the alternate
/// identifier it names, the entry it is, and the path into its occurrences.
#[test]
fn a_group_held_as_a_column_reads_as_its_run() {
    let (registry, reader) = reader();
    let parsed = reader
        .sole_line(b"8=FIX.4.4|35=AE|1907=1|1903=RTID-1|1906=0|10=0|")
        .expect("a report stating its regulatory trade identifier");
    let (root, row) = super::restatable(&registry, &parsed, &[35]);
    let at = root
        .index_of("regulatorytradeids")
        .expect("the group's column");
    let row = super::with_column_at(&row, at, &super::item_of(&root.fields()[at]));
    let message = FixMsg::with_registry(Arc::clone(&registry), root, row).expect("a message");
    assert!(super::holds_column(&message, "regulatorytradeids"));

    assert_eq!(
        message
            .get_identifiers()
            .get_from(&IdKey::base(IdType::RegTradeId)),
        Some("RTID-1")
    );
    let group = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 1907)
        .expect("the group's entry");
    assert_eq!(group.value(), Some("1"));
    assert_eq!(group.entries().len(), 1);
    assert_eq!(
        message
            .get_by_path(&super::path("regulatorytradeids[-1].regulatorytradeid"))
            .as_ref()
            .and_then(Scalar::as_str),
        Some("RTID-1")
    );
}

/// The regulatory group dates and times the message it states whether the
/// row holds it as a run or as a column.
#[test]
fn a_regulatory_group_held_as_a_column_dates_the_message() {
    /// `20260102-10:15:29.990` UTC, the execution the group stamps.
    const EXECUTION: i64 = 1_767_348_929_990_000_000;
    let (registry, reader) = reader();
    let parsed = reader
        .sole_line(
            b"8=FIX.4.4|35=AE|52=20260102-10:15:30|768=1|769=20260102-10:15:29.990|770=1|10=0|",
        )
        .expect("a report stamping its execution");
    assert_eq!(parsed.get_execunix(), Some(EXECUTION));
    assert_eq!(parsed.get_currunix(), EXECUTION);
    let (root, row) = super::restatable(&registry, &parsed, &[35, 52]);
    let run = FixMsg::with_registry(Arc::clone(&registry), root.clone(), row.clone())
        .expect("the run-backed message");
    let at = root
        .index_of("trdregtimestamps")
        .expect("the group's column");
    let row = super::with_column_at(&row, at, &super::item_of(&root.fields()[at]));
    let column =
        FixMsg::with_registry(Arc::clone(&registry), root, row).expect("the column-backed message");
    assert!(super::holds_column(&column, "trdregtimestamps"));

    assert_eq!(run.get_execunix(), Some(EXECUTION));
    assert_eq!(column.get_execunix(), Some(EXECUTION));
    assert_eq!(run.get_currunix(), EXECUTION);
    assert_eq!(column.get_currunix(), EXECUTION);
}

/// A quote's `BidPx(132)`, `BidSize(134)`, `OfferPx(133)` and
/// `OfferSize(135)` are the message's fields and no market fact of it: a
/// quote stating one side of them and no `Side(54)` names no side and no
/// price, and a stated side is the message's own.
#[test]
fn a_quotes_bid_and_offer_name_no_side_and_no_price() {
    let (_registry, reader) = reader();
    let decimal = |text: &str| Decimal::parse(text).expect("a decimal");

    let bid = reader
        .sole_line(
            b"8=FIX.4.4|35=S|52=20240102-10:15:30|117=Q1|55=AAPL|15=USD|132=101.5|134=200|10=0|",
        )
        .unwrap();
    assert_eq!(bid.get_side().as_str(), "UKNW");
    assert_eq!((bid.get_price(), bid.get_quantity()), (None, None));
    assert_eq!(bid.get_currency().as_str(), "USD");
    let wire = bid.into_bytes(b'|');
    assert!(
        !wire.windows(4).any(|held| held == b"|54="),
        "{}",
        String::from_utf8_lossy(&wire)
    );

    let stated = reader
        .sole_line(b"8=FIX.4.4|35=S|52=20240102-10:15:30|117=Q4|55=AAPL|54=2|132=101|134=10|10=0|")
        .unwrap();
    assert_eq!(stated.get_side().as_str(), "SELL");
    assert_eq!(stated.get_price(), None);
    // The side a quote states is a tag: its code stores none, and its bid
    // stands beside the ask it tags.
    assert_eq!(stated.get_crosscode(), "14:0:Q4");
    assert_eq!(stated.get_bidpx(), Some(decimal("101")));

    // A report pricing itself is about the fill: what it last executed at
    // is `lastpx`, never the price.
    let fill = reader
        .sole_line(
            b"8=FIX.4.4|35=8|52=20240102-10:15:30|37=O|17=E|150=F|39=2|31=100|32=10|132=99|10=0|",
        )
        .unwrap();
    assert_eq!(fill.get_side().as_str(), "UKNW");
    // What a filled order has left open - nothing - is its quantity.
    assert_eq!(fill.get_leavesqty(), Some(Decimal::from_int(0)));
    assert_eq!(
        (fill.get_price(), fill.get_quantity()),
        (None, Some(Decimal::from_int(0)))
    );
    assert_eq!(fill.get_lastpx(), Some(decimal("100")));
    assert_eq!(fill.get_lastqty(), Some(Decimal::from_int(10)));

    // A side a write takes away is gone.
    let mut order = reader
        .sole_line(
            b"8=FIX.4.4|35=D|52=20240102-10:15:30|11=C1|55=AAPL|15=USD|54=1|44=100|38=10|10=0|",
        )
        .unwrap();
    assert_eq!(order.get_side().as_str(), "BUYS");
    order.remove(54).unwrap();
    assert_eq!(order.get_side().as_str(), "UKNW");
}

/// The market facts an element completes, by name: what the twin property
/// compares.
fn completed_market(element: &impl Operation) -> Vec<(&'static str, Option<String>)> {
    let shown = |value: Option<Decimal>| value.map(|held| held.to_string());
    vec![
        ("currency", Some(element.get_currency().as_str().to_owned())),
        ("price", shown(element.get_price())),
        ("quantity", shown(element.get_quantity())),
        ("displayqty", shown(element.get_displayqty())),
        ("hiddenqty", shown(element.get_hiddenqty())),
        ("bidpx", shown(element.get_bidpx())),
        ("bidqty", shown(element.get_bidqty())),
        (
            "bidccy",
            element.get_bidccy().map(|held| held.as_str().to_owned()),
        ),
        ("askpx", shown(element.get_askpx())),
        ("askqty", shown(element.get_askqty())),
        (
            "askccy",
            element.get_askccy().map(|held| held.as_str().to_owned()),
        ),
        ("ordqty", shown(element.get_ordqty())),
        ("cumqty", shown(element.get_cumqty())),
        ("leavesqty", shown(element.get_leavesqty())),
        ("cxlqty", shown(element.get_cxlqty())),
        ("lastpx", shown(element.get_lastpx())),
        ("lastqty", shown(element.get_lastqty())),
        ("avgpx", shown(element.get_avgpx())),
        ("spotrate", shown(element.get_spotrate())),
        ("forwardpoints", shown(element.get_forwardpoints())),
    ]
}

/// The typed twin of `message`: an element of its category stating, through
/// the setters, its state, its side, its ticker and each market fact the
/// line states as it states it - and nothing the dictionary derived from
/// them - then finalized, as a parse finalizes the message.
fn typed_twin<E: Event + Operation>(
    mut twin: E,
    message: &FixMsg,
    line: &str,
) -> Vec<(&'static str, Option<String>)> {
    twin.set_state(*message.get_state());
    for (tag, value) in line
        .split('|')
        .filter_map(|field| field.split_once('='))
        .filter_map(|(tag, value)| Some((tag.parse::<i32>().ok()?, value)))
    {
        let number = || Some(Decimal::parse(value).expect("a decimal"));
        match tag {
            54 => twin.set_side(message.get_side(), true),
            55 => twin.set_ticker(Some(value.into()), true),
            15 => twin.set_currency(yggdryl::Ccy::new(value).expect("a currency"), true),
            44 => twin.set_price(number(), true),
            53 => twin.set_quantity(number(), true),
            1138 | 111 => twin.set_displayqty(number(), true),
            132 => twin.set_bidpx(number(), true),
            134 => twin.set_bidqty(number(), true),
            133 => twin.set_askpx(number(), true),
            135 => twin.set_askqty(number(), true),
            38 => twin.set_ordqty(number(), true),
            14 => twin.set_cumqty(number(), true),
            151 => twin.set_leavesqty(number(), true),
            84 => twin.set_cxlqty(number(), true),
            31 => twin.set_lastpx(number(), true),
            32 => twin.set_lastqty(number(), true),
            6 => twin.set_avgpx(number(), true),
            194 => twin.set_spotrate(number(), true),
            195 => twin.set_forwardpoints(number(), true),
            _ => {}
        }
    }
    twin.finalize();
    completed_market(&twin)
}

/// A FIX message and its typed twin are two representations of one market:
/// what the message completes - the dictionary's derivations over its
/// fields, then the facts it states through the setters - is exactly what
/// the twin completes stating the same raw facts through the setters alone,
/// so neither layer fills a fact the other leaves, or fills it otherwise.
#[test]
fn a_fix_message_and_its_typed_twin_complete_alike() {
    use yggdryl::MarketDataKind;
    use yggdryl::graph::{ExecutionEvent, OrderEvent, QuoteEvent};

    const CORPUS: [&str; 17] = [
        // A new order, and an iceberg showing part of it.
        "8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|44=100|38=100|40=2|15=USD|10=0|",
        "8=FIX.4.4|35=D|11=C2|55=AAPL|54=2|44=50|38=1000|1138=100|40=2|15=USD|10=0|",
        // Acknowledged, pending and accepted for bidding.
        "8=FIX.4.4|35=8|37=O1|17=E1|150=0|39=0|54=1|55=AAPL|44=10|38=100|151=100|14=0|10=0|",
        "8=FIX.4.4|35=8|37=O1|17=E2|150=A|39=A|54=1|55=AAPL|38=100|14=0|10=0|",
        "8=FIX.4.4|35=8|37=O1|17=E3|150=D|39=D|54=1|55=AAPL|38=100|10=0|",
        // Partly filled, stating what traded or what is left.
        "8=FIX.4.4|35=8|37=O2|17=E4|150=F|39=1|54=1|55=AAPL|44=100|38=100|14=40|32=40|31=100.5|15=USD|10=0|",
        "8=FIX.4.4|35=8|37=O2|17=E5|150=F|39=1|54=1|55=AAPL|38=100|151=60|32=40|31=10|10=0|",
        // Filled, and an FX forward filled at spot plus points - stating
        // the currency its symbol's pair would otherwise name, a reading of
        // the wire the twin has no symbol detector for.
        "8=FIX.4.4|35=8|37=O3|17=E6|150=F|39=2|54=2|55=AAPL|38=100|14=100|32=60|31=101|6=100.8|10=0|",
        "8=FIX.4.4|35=8|37=O4|17=E7|150=F|39=2|54=1|55=EURUSD|15=EUR|38=1000000|32=1000000|194=1.25|195=0.0025|10=0|",
        // Canceled, stating what was canceled or nothing, and expired.
        "8=FIX.4.4|35=8|37=O5|17=E8|150=4|39=4|54=1|55=AAPL|38=100|14=40|84=60|10=0|",
        "8=FIX.4.4|35=8|37=O5|17=E9|150=4|39=4|54=1|55=AAPL|38=100|10=0|",
        "8=FIX.4.4|35=8|37=O6|17=E10|150=C|39=C|54=2|55=AAPL|38=100|14=30|10=0|",
        // Rejected.
        "8=FIX.4.4|35=8|37=O7|17=E11|150=8|39=8|54=1|55=AAPL|38=100|10=0|",
        // Quotes: two-sided, tagged, and one leg alone.
        "8=FIX.4.4|35=S|117=Q1|55=AAPL|15=USD|132=99|134=7|133=101|135=8|10=0|",
        "8=FIX.4.4|35=S|117=Q2|55=AAPL|54=1|15=USD|132=99|134=7|133=101|135=8|10=0|",
        "8=FIX.4.4|35=S|117=Q3|55=AAPL|54=2|133=101|135=8|10=0|",
        // A quote's FX parts are its legs', which no last price reads.
        "8=FIX.4.4|35=S|117=Q4|55=EURUSD|15=EUR|54=1|132=1.2|134=1000000|188=1.19|189=0.01|10=0|",
    ];
    let (_, reader) = reader();
    for line in CORPUS {
        let message = reader.sole_line(line.as_bytes()).expect("a message");
        let twin = match message.marketdatakind() {
            MarketDataKind::Order => typed_twin(OrderEvent::at(0), &message, line),
            MarketDataKind::Quotation => typed_twin(QuoteEvent::at(0), &message, line),
            MarketDataKind::Execution => typed_twin(ExecutionEvent::at(0), &message, line),
            other => panic!("{line}: an order, a quote or an execution, got {other:?}"),
        };
        assert_eq!(completed_market(&message), twin, "{line}");
    }
}

mod market_ladder {
    //! The market a message names, and what a bridge's instrument key fills.

    use yggdryl::graph::Market;
    use yggdryl::{FixMsg, IdType};

    fn parsed(line: &str) -> FixMsg {
        super::super::fixed_codec(super::super::committed_registry())
            .parse_fix_line(line.as_bytes())
            .expect("a readable line")
    }

    fn mic(line: &str) -> Option<String> {
        parsed(line)
            .get_miccode()
            .map(|held| held.as_str().to_owned())
    }

    const HEAD: &str = "8=FIX.4.4|35=8|17=E1|55=HOLN|";

    #[test]
    fn the_market_is_the_first_iso_mic_down_the_ladder() {
        for (tail, expected) in [
            ("30=XLON|100=XPAR|207=XSWX|", Some("XLON")),
            ("100=XPAR|207=XSWX|", Some("XPAR")),
            ("207=XSWX|", Some("XSWX")),
            // A Reuters mnemonic, FIX 4.2's spelling, names its market.
            ("30=L|100=XPAR|207=XSWX|", Some("XLON")),
            ("100=TW|207=XTAI|", Some("XTAI")),
            ("207=S|", Some("XSWX")),
            // A code neither reading resolves names none: the next step answers.
            ("30=ZZ|100=TH|207=XSWX|", Some("XSWX")),
            ("30=xlon|", None),
            // The instrument key's market ranks before SecurityExchange.
            (
                "207=XSWX|OMSINSTRUMENTID=dbi;CH0012214059_XETR_CHF|",
                Some("XETR"),
            ),
            (
                "100=S|OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|",
                Some("XSWX"),
            ),
            (
                "30=XLON|OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|",
                Some("XLON"),
            ),
            ("", None),
        ] {
            let line = format!("{HEAD}{tail}10=0|");
            assert_eq!(mic(&line).as_deref(), expected, "{line}");
        }
    }

    #[test]
    fn a_market_the_row_states_wins_over_the_ladder() {
        for (tail, expected) in [
            ("MICCODE=XAMS|30=XLON|", "XAMS"),
            ("INSTRUMENT[EXCHANGE]=XSWX|30=XLON|", "XSWX"),
        ] {
            let line = format!("{HEAD}{tail}10=0|");
            let held = parsed(&line);
            assert_eq!(
                held.get_miccode().map(|held| held.as_str()),
                Some(expected),
                "{line}"
            );
            assert!(
                held.anomalies().is_empty(),
                "{line}: {:?}",
                held.anomalies()
            );
        }
    }

    #[test]
    fn a_short_code_under_instrument_exchange_is_an_anomaly_and_the_ladder_answers() {
        for code in ["S", "TW"] {
            let line = format!("{HEAD}INSTRUMENT[EXCHANGE]={code}|207=XSWX|10=0|");
            let held = parsed(&line);
            assert_eq!(held.get_miccode().map(|held| held.as_str()), Some("XSWX"));
            let anomalies: Vec<(&str, &str)> = held
                .anomalies()
                .iter()
                .map(|held| (held.field(), held.reason()))
                .collect();
            assert_eq!(anomalies.len(), 1, "{anomalies:?}");
            assert_eq!(anomalies[0].0, "instrument[exchange]");
            assert!(anomalies[0].1.contains(code), "{}", anomalies[0].1);
            // The spelling names the market, which holds another value: it
            // is kept in the metadata and never re-emitted, the wire
            // carrying the dictionary's own fields alone.
            assert_eq!(
                held.metadata()
                    .get("instrument[exchange]")
                    .map(|held| held.as_str()),
                Some(code)
            );
            let wire = String::from_utf8(held.into_bytes(b'|')).expect("a text wire");
            assert!(
                !wire.contains("instrument[exchange]"),
                "the spelling is no field of the wire: {wire}"
            );
            // A row carries it in its metadata, and reads it back there
            // rather than as the market.
            let registry = super::super::committed_registry();
            let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
            let row = held.into_row(&schema).expect("a row");
            let again =
                FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &row).expect("again");
            assert_eq!(again.get_miccode().map(|held| held.as_str()), Some("XSWX"));
            assert_eq!(
                again
                    .metadata()
                    .get("instrument[exchange]")
                    .map(|held| held.as_str()),
                Some(code)
            );
        }
    }

    #[test]
    fn the_instrument_key_fills_the_isin_and_the_currency_only_where_open() {
        let held = parsed(&format!(
            "{HEAD}OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|10=0|"
        ));
        assert_eq!(
            held.get_securityids().get(&IdType::Isin),
            Some("CH0012214059")
        );
        assert_eq!(held.get_currency().as_str(), "CHF");
        let wire = String::from_utf8(held.into_bytes(b'|')).expect("a text wire");
        assert!(
            !wire.contains("|48=") && !wire.contains("|15="),
            "nothing is written back: {wire}"
        );

        // What the fields state stands.
        let held = parsed(&format!(
            "{HEAD}15=EUR|22=4|48=CH0012221716|OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|10=0|"
        ));
        assert_eq!(
            held.get_securityids().get(&IdType::Isin),
            Some("CH0012221716")
        );
        assert_eq!(held.get_currency().as_str(), "EUR");

        // A part its type refuses is skipped, and the others still answer.
        let held = parsed(&format!(
            "{HEAD}OMSINSTRUMENTID=dbi;CH0012214058_XSWX_CHF|10=0|"
        ));
        assert_eq!(held.get_securityids().get(&IdType::Isin), None);
        assert_eq!(held.get_currency().as_str(), "CHF");
        assert_eq!(held.get_miccode().map(|held| held.as_str()), Some("XSWX"));

        // The currency part is a currency of up to eight bytes, a
        // digital-asset ticker as well as ISO 4217's three letters.
        let held = parsed(&format!(
            "{HEAD}OMSINSTRUMENTID=dbi;CH0012214059_XSWX_USDT|10=0|"
        ));
        assert_eq!(
            held.get_securityids().get(&IdType::Isin),
            Some("CH0012214059")
        );
        assert_eq!(held.get_currency().as_str(), "USDT");

        // A value not shaped twelve, four and three to eight names nothing.
        for code in [
            "dbi;CH0012214059_XSWX_TOOLONGCCY",
            "dbi;CH001221405_XSWX_CHF",
            "dbi;CH0012214059_XSWX",
            "dbi;CH0012214059_XSWX_CHF_X",
            "CH0012214059-XSWX-CHF",
        ] {
            let held = parsed(&format!("{HEAD}OMSINSTRUMENTID={code}|10=0|"));
            assert_eq!(held.get_securityids().get(&IdType::Isin), None, "{code}");
            assert!(held.get_miccode().is_none(), "{code}");
        }
    }
}

mod identifier_maps {
    //! The identifiers a message rebuilds from the dictionary's
    //! `FIX:idmap` sources at every settle, the ones a bridge's own keys
    //! name, and the parties its `Parties` occurrences, its `Account(1)`
    //! and a bridge's user and account keys name, which leave its leaves'
    //! metadata.

    use yggdryl::graph::{Element, Market, Operation};
    use yggdryl::{FixMsg, IdType, Identifier};

    fn parsed(line: &str) -> FixMsg {
        super::super::fixed_codec(super::super::committed_registry())
            .parse_fix_line(line.as_bytes())
            .expect("a readable line")
    }

    /// Every identifier of a set as `src:type=value`.
    fn shown(ids: &yggdryl::Identifiers) -> Vec<String> {
        ids.iter().map(ToString::to_string).collect()
    }

    /// An identifier of `kind` the wire's own fields state.
    fn fix_id(kind: &str, value: &str) -> Identifier {
        Identifier::new(yggdryl::IdKey::base(kind.parse().expect("a type")), value)
            .expect("an identifier")
    }

    /// The metadata of the first leaf the message expands to: what it
    /// states that no typed column reads.
    fn leaf_metadata(message: &FixMsg) -> yggdryl::graph::Metadata {
        message.market_data().expect("market data")[0]
            .get_metadata()
            .clone()
    }

    /// A bridge's code-like keys each land in `securityids`: the crate's
    /// own `ISINCODE` and `BLOOMBERGCODE` views, and the keyed `RICCODE`,
    /// `OMS_CUSIPCODE` and `SEDOLCODE` the alias table reads. The row's
    /// `metadata` holds none of them - a keyed one rides `fixentries` under
    /// `0:<key>` as it arrived, a view its own column - and only a value a
    /// type refuses by shape stays in `metadata`; the row read back restates
    /// the wire, the sets, the digest and the identity.
    #[test]
    fn a_bridges_code_aliases_land_in_securityids_and_leave_the_rows_metadata() {
        let held = parsed(
            "8=FIX.4.4|35=D|11=C1|ISINCODE=US0378331005|BLOOMBERGCODE=AAPL US Equity|\
             OMS_CUSIPCODE=037833100|RICCODE=AAPL.O|SEDOLCODE=2046251|#CUSIPCODE=03783310|\
             10=0|",
        );
        let ids = held.get_securityids();
        for (kind, value) in [
            (IdType::Isin, "US0378331005"),
            (IdType::Bloomberg, "AAPL US Equity"),
            (IdType::Ric, "AAPL.O"),
            (IdType::Cusip, "037833100"),
            (IdType::Sedol, "2046251"),
        ] {
            assert_eq!(ids.get(&kind), Some(value), "{kind}: {ids}");
        }
        assert_eq!(
            ids.get_from(&yggdryl::IdKey::new("oms".parse().unwrap(), IdType::Cusip)),
            Some("037833100")
        );
        let anomalies: Vec<&str> = held.anomalies().iter().map(|held| held.field()).collect();
        assert_eq!(anomalies, ["cusipcode"]);

        let registry = super::super::committed_registry();
        let schema = yggdryl::fix_schema(&registry, "fix").expect("a schema");
        let row = held.into_row(&schema).expect("a row");
        let cells = row.as_sequence().expect("a row");
        let cell = |name: &str| -> Vec<(String, String)> {
            cells[schema.index_of(name).expect(name)]
                .as_mapping()
                .map(|held| {
                    held.iter()
                        .map(|(key, value)| {
                            (
                                key.as_str().expect("a text key").to_owned(),
                                value.as_str().expect("a text value").to_owned(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        assert_eq!(
            cell("metadata"),
            [("cusipcode".to_owned(), "03783310".to_owned())]
        );
        let residual = cell("fixentries");
        for (key, value) in [
            ("0:omscusipcode", "037833100"),
            ("0:riccode", "AAPL.O"),
            ("0:sedolcode", "2046251"),
        ] {
            assert!(
                residual.contains(&(key.to_owned(), value.to_owned())),
                "{key} in {residual:?}"
            );
        }
        assert!(
            residual.iter().all(|(key, _)| {
                key != "0:cusipcode"
                    && !key.ends_with("isincode")
                    && !key.ends_with("bloombergcode")
            }),
            "{residual:?}"
        );
        let column = |name: &str| cells[schema.index_of(name).expect(name)].as_str();
        assert_eq!(column("isincode"), Some("US0378331005"));
        assert_eq!(column("bloombergcode"), Some("AAPL US Equity"));

        let again =
            FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &row).expect("again");
        assert_eq!(again.get_securityids(), ids);
        assert_eq!(again.metadata(), held.metadata());
        let tokens = |held: &FixMsg| {
            let mut tokens: Vec<String> = String::from_utf8(held.into_bytes(b'|'))
                .expect("a text wire")
                .split('|')
                .map(str::to_owned)
                .collect();
            tokens.sort();
            tokens
        };
        assert_eq!(tokens(&again), tokens(&held));
        assert_eq!(again.digest(), held.digest());
        assert_eq!(again.get_currhashcode(), held.get_currhashcode());
        assert_eq!(again.into_row(&schema).expect("a row again"), row);
    }

    #[test]
    fn a_bridges_spellings_of_fix_fields_are_those_fields_and_its_own_keys_name_the_message() {
        let held = parsed(
            "8=FIX.4.4|35=8|17=E1|37=O1|150=F|39=2|54=1|55=AAPL|31=10|32=1|1=ACC|\
             OMSDEALERACCOUNT=YNHD5|OMSUSERID=trader1|\
             PARENTORDERID=P1|PARENTCLORDID=PC1|OMSDEALERPARENTORDERID=OP1|\
             EXCHANGECLIENTORDERID=X1|TRANSVERSAL_KEY=T1|ULTRADER_CLORDID=U1|10=0|",
        );
        // The dictionary names three of the spellings:
        // `exchangeclientorderid` SecondaryClOrdID(526), `ultraderclordid`
        // ClOrdID(11) and `omsuserid` Username(553), each the field it
        // names. The other keys are read as the identifier they name: the
        // source is the rest of the key, the type the identifier name it
        // ends with, a parentage word kept inside the type - so a bridge's
        // `PARENTCLORDID`, a hierarchy parent and never OrigClOrdID(41), is
        // the word `parentclordid`, a type of its own; a parent the message
        // states without its base states the base, under its own source.
        assert_eq!(
            shown(held.get_identifiers()),
            [
                "clordid=U1",
                "execid=E1",
                "omsdealer:orderid=OP1",
                "omsdealer:parentorderid=OP1",
                "orderid=O1",
                "parentclordid=PC1",
                "parentorderid=P1",
                "secondaryclordid=X1",
            ]
        );
        let text = |tag: i32| {
            held.get_by_tag(tag)
                .and_then(|value| value.as_str().map(str::to_owned))
        };
        assert_eq!(text(41), None, "no OrigClOrdID(41)");
        for (tag, value) in [(11, "U1"), (526, "X1"), (553, "trader1"), (1, "ACC")] {
            assert_eq!(text(tag).as_deref(), Some(value), "{tag}");
        }
        // `OMSDEALERACCOUNT` names `Account(1)`, which states another
        // account: the message keeps it in its metadata beside an anomaly,
        // and it is read off its key as the dealer's account party.
        assert_eq!(
            held.metadata()
                .get("omsdealeraccount")
                .map(|held| held.as_str()),
            Some("YNHD5")
        );
        let anomalies: Vec<&str> = held.anomalies().iter().map(|held| held.field()).collect();
        assert_eq!(anomalies, ["omsdealeraccount"]);
        // Content, as it arrived, and no column of the fixed row.
        let registry = super::super::committed_registry();
        let schema = yggdryl::fix_schema(&registry, "fix").expect("a schema");
        for (name, value) in [
            ("omsdealerparentorderid", "OP1"),
            ("parentclordid", "PC1"),
            ("parentorderid", "P1"),
            ("transversalkey", "T1"),
        ] {
            assert_eq!(
                held.get_by_name(name)
                    .and_then(|held| held.as_str().map(str::to_owned))
                    .as_deref(),
                Some(value),
                "{name}"
            );
            assert!(schema.index_of(name).is_none(), "{name} is no column");
        }
        // An account and a user are parties, never identifiers.
        for key in ["account", "userid"] {
            assert!(
                !held
                    .get_identifiers()
                    .contains_kind(&key.parse::<IdType>().unwrap()),
                "{key}: {:?}",
                shown(held.get_identifiers())
            );
        }
        assert_eq!(
            held.get_partyids().to_string(),
            "[account=ACC, omsdealer:account=YNHD5]"
        );
        // The leaf holds what it read in its sets and drops it from its
        // metadata: only the key no identifier name ends is still there.
        let metadata = leaf_metadata(&held);
        for key in [
            "omsdealeraccount",
            "parentorderid",
            "omsdealerparentorderid",
        ] {
            assert!(!metadata.contains_key(key), "{key}: {metadata:?}");
        }
        assert_eq!(
            metadata.get("transversalkey").map(|held| held.as_str()),
            Some("T1")
        );
        let wire = String::from_utf8(held.into_bytes(b'|')).expect("a text wire");
        for pair in [
            "|11=U1|",
            "|parentclordid=PC1|",
            "|526=X1|",
            "|553=trader1|",
            "|1=ACC|",
        ] {
            assert!(wire.contains(pair), "{pair} in {wire}");
        }
        assert!(!wire.contains("|41="), "{wire}");
        assert!(!wire.contains("YNHD5"), "{wire}");
        // A statement under a type and source held already fills nothing,
        // and the wire stays as the source sent it.
        let mut held = held;
        assert!(
            held.insert_identifier(fix_id("execid", "other"))
                .expect("a stated key")
                .eq(&false)
        );
        assert_eq!(held.get_identifiers().get(&IdType::ExecId), Some("E1"));
        let wire = String::from_utf8(held.into_bytes(b'|')).expect("a text wire");
        assert!(
            wire.contains("|17=E1|") && !wire.contains("other"),
            "{wire}"
        );
    }

    /// A bridge's party key whose namespace is the `bic` standard holds its
    /// value as a BIC, upper-cased, and the leaf drops the key from its
    /// metadata as it drops every key a map holds; a value that is no BIC is
    /// held by no map and stays in the metadata as it arrived.
    #[test]
    fn a_bridges_bic_keyed_party_is_held_as_a_bic_and_leaves_the_metadata() {
        let user = yggdryl::IdKey::new(yggdryl::IdSource::Bic, IdType::UserId);
        let held = parsed("8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=5|40=2|BIC_UserID=deutdeff|10=0|");
        assert_eq!(held.get_partyids().get_from(&user), Some("DEUTDEFF"));
        let metadata = leaf_metadata(&held);
        assert!(
            !metadata.iter().any(|(key, _)| key.contains("userid")),
            "{metadata:?}"
        );
        let refused = parsed("8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=5|40=2|BIC_UserID=T-1|10=0|");
        assert!(refused.get_partyids().get_from(&user).is_none());
        let metadata = leaf_metadata(&refused);
        assert!(
            metadata
                .iter()
                .any(|(key, value)| key.contains("userid") && value == "T-1"),
            "{metadata:?}"
        );
    }

    /// `FinancialInstrumentShortName(2737)` states the instrument's ISO
    /// 18774 short name, which FIX gives no `SecurityIDSource(22)` code: it
    /// lands in `securityids` under the base `fisn` key, upper-cased, beside
    /// the primary the message states; one that is no short name is an
    /// anomaly of the field, which stays on the wire as it arrived.
    #[test]
    fn a_financial_instrument_short_name_lands_in_securityids_as_its_fisn() {
        let held = parsed(
            "8=FIX.4.4|35=D|11=C1|55=AAPL|48=US0378331005|22=4|2737=apple inc/sh|54=1|38=5|\
             40=2|10=0|",
        );
        let ids = held.get_securityids();
        assert_eq!(ids.get(&IdType::Fisn), Some("APPLE INC/SH"));
        assert_eq!(ids.get(&IdType::Isin), Some("US0378331005"));
        assert_eq!(ids.of_kind(&IdType::Fisn).count(), 1, "the base key alone");
        assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
        let wire = String::from_utf8(held.into_bytes(b'|')).expect("a text wire");
        assert!(wire.contains("|2737=apple inc/sh|"), "{wire}");

        let refused = parsed(
            "8=FIX.4.4|35=D|11=C1|55=AAPL|48=US0378331005|22=4|2737=APPLE INC SH|54=1|38=5|\
             40=2|10=0|",
        );
        assert!(refused.get_securityids().get(&IdType::Fisn).is_none());
        assert_eq!(
            refused.get_securityids().get(&IdType::Isin),
            Some("US0378331005")
        );
        let anomalies = refused.anomalies();
        assert_eq!(anomalies.len(), 1, "{anomalies:?}");
        assert_eq!(anomalies[0].field(), "financialinstrumentshortname");
        assert!(
            anomalies[0].reason().contains("'/'"),
            "{}",
            anomalies[0].reason()
        );
        let wire = String::from_utf8(refused.into_bytes(b'|')).expect("a text wire");
        assert!(wire.contains("|2737=APPLE INC SH|"), "{wire}");

        // A message stating the short name alone holds it as its one
        // security identifier, and a null-like one states nothing.
        let alone = parsed("8=FIX.4.4|35=D|11=C1|55=AAPL|2737=APPLE INC/SH|54=1|38=5|40=2|10=0|");
        assert_eq!(
            alone.get_securityids().get(&IdType::Fisn),
            Some("APPLE INC/SH")
        );
        let none = parsed("8=FIX.4.4|35=D|11=C1|55=AAPL|2737=N/A|54=1|38=5|40=2|10=0|");
        assert!(none.get_securityids().get(&IdType::Fisn).is_none());
        assert!(none.anomalies().is_empty(), "{:?}", none.anomalies());
    }

    #[test]
    fn the_parties_a_message_names_keep_the_first_of_a_role_and_a_second_stays() {
        let held = parsed(
            "8=FIX.4.4|35=8|17=E1|37=O1|150=F|39=2|54=1|55=AAPL|31=10|32=1|453=3|448=T1|\
             447=D|452=36|448=T2|447=D|452=36|\
             448=C1|447=D|452=24|10=0|",
        );
        // The first party of each role and source is a party; the second
        // trader of one role is ordinary, no anomaly, and it alone stays in
        // the leaf's metadata, since no map the leaf holds names it.
        assert_eq!(
            held.get_partyids().to_string(),
            "[customeraccount=C1, enteringtrader=T1, proprietary:customeraccount=C1, proprietary:enteringtrader=T1]"
        );
        let metadata = leaf_metadata(&held);
        assert_eq!(
            metadata.get("parties").map(|held| held.as_str()),
            Some(r#"[{"partyid":"T2","partyidsource":"D","partyrole":"36"}]"#)
        );
        // No party is an identifier, so none is an anomaly.
        assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
        assert!(
            held.get_identifiers()
                .iter()
                .all(|id| id.kind() == "orderid" || id.kind() == "execid"),
            "{:?}",
            held.get_identifiers()
        );
    }

    /// `PartyRole(452)` `21` is `ClearingOrganization` in the code set: its
    /// name folded is the party's type - a party role - and never
    /// `SecurityIDSource(22)` `H`'s `clearinghouse`, a security type.
    #[test]
    fn a_clearing_organization_party_is_typed_by_its_roles_own_name_and_is_no_security() {
        let held = parsed(
            "8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=5|40=2|453=2|448=LCH|447=D|452=21|\
             448=T1|447=D|452=12|10=0|",
        );
        assert_eq!(
            shown(held.get_partyids()),
            [
                "clearingorganization=LCH",
                "executingtrader=T1",
                "proprietary:clearingorganization=LCH",
                "proprietary:executingtrader=T1"
            ]
        );
        assert!(
            held.get_partyids()
                .iter()
                .all(|id| id.kind().is_party() && !id.kind().is_security()),
            "{}",
            held.get_partyids()
        );
        // A party's value is any text, whatever its role: a security type's
        // printable-ASCII rule never reaches it.
        let held = parsed(
            "8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=5|40=2|453=1|448=Chambre\u{e9}|447=D|452=21|10=0|",
        );
        assert_eq!(
            shown(held.get_partyids()),
            [
                "clearingorganization=Chambre\u{e9}",
                "proprietary:clearingorganization=Chambre\u{e9}"
            ]
        );
    }

    /// A source the set names nothing for is its own spelling where it is a
    /// word; a bare wire code - one character, or digits - is prefixed by
    /// its field, as a role is, so it never reads as a word of its own.
    #[test]
    fn a_party_or_account_source_the_set_names_nothing_for_is_its_spelling_or_its_prefixed_code() {
        let held = parsed(
            "8=FIX.4.4|35=D|11=C1|1=ACC|660=7|55=AAPL|54=1|40=2|453=2|448=T1|447=W|452=12|\
             448=F1|447=MyVenue|452=1|10=0|",
        );
        assert_eq!(
            shown(held.get_partyids()),
            [
                "account=ACC",
                "acctidsource7:account=ACC",
                "executingfirm=F1",
                "executingtrader=T1",
                "myvenue:executingfirm=F1",
                "partyidsourcew:executingtrader=T1",
            ]
        );
    }

    /// `Account(1)` is a party typed `account`, sourced by its
    /// `AcctIDSource(660)` code's name; each `RootPartyID(1117)` of
    /// `NoRootPartyIDs(1116)` is typed by its `RootPartyRole(1119)` and
    /// sourced by its `RootPartyIDSource(1118)`, `base` for none.
    #[test]
    fn an_account_is_sourced_by_its_acctidsource_name_and_root_parties_are_parties() {
        for (account, source, expected) in [
            (
                "deutdeff",
                "1",
                ["account=DEUTDEFF", "bic:account=DEUTDEFF"],
            ),
            ("ACC-1", "6", ["account=ACC-1", "spsaid:account=ACC-1"]),
            ("ACC-1", "99", ["account=ACC-1", "other:account=ACC-1"]),
        ] {
            let held = parsed(&format!(
                "8=FIX.4.4|35=D|11=C1|1={account}|660={source}|55=AAPL|54=1|38=5|40=2|10=0|"
            ));
            // The source fills the base key, the account's answer; a BIC
            // source holds its value as a BIC, upper-cased.
            assert_eq!(shown(held.get_partyids()), expected, "660={source}");
            assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
        }
        let quoted = parsed(
            "8=FIX.4.4|35=R|131=QR1|303=2|55=AAPL|1116=2|1117=R1|1118=D|1119=12|1117=R2|1119=3|10=0|",
        );
        assert_eq!(
            shown(quoted.get_partyids()),
            [
                "clientid=R2",
                "executingtrader=R1",
                "proprietary:executingtrader=R1"
            ]
        );
    }

    /// A party sourced `B` by `PartyIDSource(447)` is a BIC and one sourced
    /// `N` an LEI, as `AcctIDSource(660)` `1` makes an account a BIC: a
    /// value of the code's shape is held upper-cased under the source's key
    /// and fills the role's answer, and one that is not is no party - kept
    /// on the wire and in the leaf's metadata, and recorded as an anomaly
    /// of its identifier field naming the key, never dropped silently.
    #[test]
    fn a_party_under_a_bic_or_lei_source_is_held_to_that_code_and_a_refusal_is_an_anomaly() {
        let held = parsed(
            "8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=5|40=2|453=2|448=deutdeff|447=B|452=1|\
             448=hwupkr0mpou8fgxbt394|447=N|452=3|10=0|",
        );
        assert_eq!(
            shown(held.get_partyids()),
            [
                "bic:executingfirm=DEUTDEFF",
                "clientid=HWUPKR0MPOU8FGXBT394",
                "executingfirm=DEUTDEFF",
                "legalentityidentifier:clientid=HWUPKR0MPOU8FGXBT394",
            ]
        );
        assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());

        let held = parsed(
            "8=FIX.4.4|35=D|11=C1|1=ACC-1|660=1|55=AAPL|54=1|38=5|40=2|453=3|448=ACME|447=B|\
             452=1|448=CL|447=N|452=3|448=T1|447=D|452=12|10=0|",
        );
        assert_eq!(
            shown(held.get_partyids()),
            ["executingtrader=T1", "proprietary:executingtrader=T1"],
            "only the proprietary party stands"
        );
        let anomalies: Vec<(String, String)> = held
            .anomalies()
            .iter()
            .map(|anomaly| (anomaly.field().to_owned(), anomaly.reason().to_owned()))
            .collect();
        assert_eq!(
            anomalies
                .iter()
                .map(|(field, _)| field.as_str())
                .collect::<Vec<_>>(),
            ["partyid", "partyid", "account"]
        );
        assert_eq!(
            anomalies[0].1,
            "states \"ACME\", which no identifier holds: invalid record value at \
             bic:executingfirm: a value under the bic source is a BIC: expected eight or eleven \
             characters, got \"ACME\""
        );
        assert!(
            anomalies[1].1.contains("legalentityidentifier:clientid")
                && anomalies[1].1.contains("an LEI"),
            "{}",
            anomalies[1].1
        );
        assert!(anomalies[2].1.contains("bic:account"), "{}", anomalies[2].1);
        // The wire is as it arrived, and the refused parties stay in the
        // leaf's metadata, since no map holds them.
        let wire = String::from_utf8(held.clone().into_bytes(b'|')).expect("a text wire");
        assert!(
            wire.contains("|448=ACME|447=B|452=1|") && wire.contains("|1=ACC-1|660=1|"),
            "{wire}"
        );
        let metadata = leaf_metadata(&held);
        let parties = metadata.get("parties").map(|held| held.as_str().to_owned());
        assert!(
            parties
                .as_deref()
                .is_some_and(|parties| parties.contains("ACME")
                    && parties.contains("\"CL\"")
                    && !parties.contains("T1")),
            "{parties:?}"
        );
        // A settle states the same anomalies again, never twice.
        let restated = parsed(&String::from_utf8(held.into_bytes(b'|')).expect("a text wire"));
        assert_eq!(restated.anomalies().len(), 3, "{:?}", restated.anomalies());
    }

    /// A regulatory trade identifier is typed by its
    /// `RegulatoryTradeIDType(1906)` alone: neither its
    /// `RegulatoryTradeIDSource(1905)` nor its
    /// `RegulatoryTradeIDScope(2397)` reaches the identifier, so a second
    /// current one - another scope's - is an anomaly and the first stands.
    #[test]
    fn a_regulatory_trade_id_is_typed_by_its_type_alone_and_a_second_scope_is_an_anomaly() {
        let held = parsed(
            "8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=2|1907=2|1903=UTI-1|1905=LEI-A|1906=0|2397=1|\
             1903=UTI-2|1905=LEI-B|1906=0|2397=2|10=0|",
        );
        assert_eq!(
            held.get_identifiers()
                .get_from(&yggdryl::IdKey::base(IdType::RegTradeId)),
            Some("UTI-1"),
            "{}",
            held.get_identifiers()
        );
        let dropped: Vec<_> = held
            .anomalies()
            .iter()
            .filter(|anomaly| anomaly.field() == "regulatorytradeids")
            .collect();
        assert_eq!(dropped.len(), 1, "{:?}", held.anomalies());
        assert!(
            dropped[0]
                .reason()
                .contains("states regtradeid=UTI-2 where regtradeid=UTI-1"),
            "{}",
            dropped[0].reason()
        );
    }

    #[test]
    fn a_following_operation_carries_what_the_dictionary_follows() {
        let held = parsed("8=FIX.4.4|35=8|17=E1|37=O1|10=0|");
        // The parents of an identifier are no flag of any field: they travel
        // with a base that follows, so a follower naming no `OrderID(37)`
        // keeps its chain's lineage, and not with one that does not.
        for key in [
            "orderid",
            "secondaryorderid",
            "parentorderid",
            "origorderid",
        ] {
            assert!(held.is_followed_identifier(&fix_id(key, "x")), "{key}");
        }
        for key in [
            "clordid",
            "execid",
            "origclordid",
            "ultraderclordid",
            "marketorderid",
        ] {
            assert!(!held.is_followed_identifier(&fix_id(key, "x")), "{key}");
        }
    }

    /// A message stating a parent but not its base is what the parent says
    /// it was: the base takes the value of the nearest parent it states,
    /// `parentorderid` before `origorderid`, under that parent's source.
    #[test]
    fn a_parent_stated_without_its_base_states_the_base_nearest_first() {
        let both =
            parsed("8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|40=2|ParentOrderID=P1|OrigOrderID=O0|10=0|");
        assert_eq!(
            shown(both.get_identifiers()),
            [
                "clordid=C1",
                "orderid=P1",
                "origorderid=O0",
                "parentorderid=P1",
            ]
        );
        let farthest = parsed("8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|40=2|OrigOrderID=O0|10=0|");
        assert_eq!(
            shown(farthest.get_identifiers()),
            ["clordid=C1", "orderid=O0", "origorderid=O0"]
        );
        // The wire's `OrderID(37)` states the type's answer, so the
        // bridge's parent fills no base the wire states.
        let beside =
            parsed("8=FIX.4.4|35=8|17=E1|37=O1|150=0|39=0|54=1|55=AAPL|ParentOrderID=P1|10=0|");
        assert_eq!(
            beside
                .get_identifiers()
                .get_from(&yggdryl::IdKey::base(IdType::OrderId)),
            Some("O1")
        );
        assert_eq!(
            beside.get_identifiers().get_from(&yggdryl::IdKey::base(
                "parentorderid".parse().expect("a type")
            )),
            Some("P1")
        );
        // An empty value states nothing, and fills nothing.
        let empty = parsed("8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|40=2|ParentOrderID=|10=0|");
        assert_eq!(shown(empty.get_identifiers()), ["clordid=C1"]);
        // A namespaced key names its own source, a dot inside kept: a parent
        // there fills that source's base, however the wire states its own,
        // and the base parent key it fills where nothing else states one.
        let stated = parsed(
            "8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|40=2|firm.x.ParentOrderID=P1|OrderID=O9|10=0|",
        );
        assert_eq!(
            shown(stated.get_identifiers()),
            [
                "clordid=C1",
                "firm.x:orderid=P1",
                "firm.x:parentorderid=P1",
                "orderid=O9",
                "parentorderid=P1",
            ]
        );
    }

    /// The registry's own list says which types are parents of a base: a
    /// dictionary field stating `grandparentorderid` under its base key is a
    /// base of its own by the names alone, and the middle of the three
    /// parents `OrderID(37)` states in the registry that lists them - which
    /// the base then takes before the farthest.
    #[test]
    fn a_registrys_list_says_which_types_are_parents_of_a_base() {
        use yggdryl::DataType;
        use yggdryl::fix::{FixIdMapKind, FixIdSource};

        const LINE: &str = "8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|40=2|9100=G1|OrigOrderID=O0|10=0|";
        let registry = |listed: bool| {
            let mut registry = (*super::super::committed_registry()).clone();
            let mut grand = DataType::utf8().nullable_field("GrandParentOrderID");
            grand.as_fix_mut().set_tag(9100).expect("a tag");
            grand
                .as_fix_mut()
                .set_idmap(&[FixIdSource::new(
                    FixIdMapKind::Identifiers,
                    "grandparentorderid".parse().expect("a type"),
                )])
                .expect("a document");
            registry.add_field(grand).expect("a field");
            if listed {
                let mut orderid = registry.field_by_tag(37).expect("OrderID(37)").clone();
                orderid
                    .as_fix_mut()
                    .set_parents(["parentorderid", "grandparentorderid", "origorderid"])
                    .expect("three types");
                registry.update(orderid).expect("the field restated");
            }
            std::sync::Arc::new(registry)
        };
        let message = |listed: bool| {
            super::super::fixed_codec(registry(listed))
                .parse_fix_line(LINE.as_bytes())
                .expect("a readable line")
        };

        let named = message(false);
        assert_eq!(
            shown(named.get_identifiers()),
            [
                "clordid=C1",
                "grandparentorderid=G1",
                "orderid=O0",
                "origorderid=O0",
            ],
            "by its name alone `grandparentorderid` is a base of its own"
        );
        let listed = message(true);
        assert_eq!(
            shown(listed.get_identifiers()),
            [
                "clordid=C1",
                "grandparentorderid=G1",
                "orderid=G1",
                "origorderid=O0",
            ],
            "the list makes it a parent of `orderid`, and the nearer of the two stated"
        );
    }
}

/// A message naming no currency pair digests exactly as it did before FX
/// detection existed: detection writes nothing where it finds no pair.
///
/// The pin moved when an event's place left its content code: the code no
/// longer feeds `seqnum`, which this message states as zero. It last moved
/// when the category the content code feeds took the column's own name:
/// the label `msgcat` became `marketdatakind`, its value the same code.
/// That relabelling moves every message's `currhashcode` and `curruuid`, a
/// `crossuuid` that is its own `curruuid`, the `srcuuids` and `prevuuid`
/// naming a moved message, and the cross code of an execution split off a
/// report naming no `ExecID` or `TradeID` (it derives from its report's
/// `currhashcode`); never the wire, the digest's entries or `seqnum`.
#[test]
fn a_message_naming_no_pair_digests_as_it_did_before_detection() {
    let (_, reader) = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|54=1|38=100|40=2|44=10.5|15=USD|167=CS|10=0|")
        .expect("an order");
    assert_eq!(message.get_currhashcode(), 11_376_276_928_047_898_508);
}

/// What settle derives about the market a message is in: the rates it
/// states, its last price's FX triple completed, and its category.
mod settled_market {
    use std::sync::Arc;

    use super::SoleMessage;
    use yggdryl::graph::{FxRates, Market, MarketData};
    use yggdryl::{Ccy, Decimal, FixMsg, MarketDataKind, Scalar, fix_schema};

    fn decimal(text: &str) -> Decimal {
        text.parse().expect("a decimal")
    }

    fn parsed(line: &str) -> FixMsg {
        let (_, reader) = super::reader();
        reader.sole_line(line.as_bytes()).expect("a message")
    }

    /// An order is typed by its `OrdType(40)`, a quote by its
    /// `QuoteType(537)`, a trade capture by its `TrdType(828)`; a value no
    /// member names reads as its set's catch-all, and nothing stated is
    /// `UKNW`.
    #[test]
    fn a_message_is_typed_by_the_field_its_kind_names() {
        use yggdryl::MarketDataType;
        let typed = |line: &str| parsed(line).get_marketdatatype();
        assert_eq!(
            typed("8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=2|44=100|38=5|10=0|"),
            MarketDataType::OrdLimit
        );
        assert_eq!(
            typed("8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=1|38=5|10=0|"),
            MarketDataType::OrdMarket
        );
        assert_eq!(
            typed("8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=Z|38=5|10=0|"),
            MarketDataType::OrdOther
        );
        assert_eq!(
            typed("8=FIX.4.4|35=S|117=Q1|55=AAPL|54=1|537=1|132=100|10=0|"),
            MarketDataType::QuoTradeable
        );
        assert_eq!(
            typed("8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=5|10=0|"),
            MarketDataType::Unknown
        );
    }

    /// A message type with a typing field of its own reads it before its
    /// kind's: a trade capture report is what the report is, a mass cancel
    /// what it cancels, a market data request what it subscribes to.
    #[test]
    fn a_message_type_reads_its_own_typing_field_first() {
        use yggdryl::MarketDataType;
        let typed = |line: &str| parsed(line).get_marketdatatype();
        assert_eq!(
            typed("8=FIX.4.4|35=AE|571=T1|856=0|828=1|55=AAPL|32=10|31=5|10=0|"),
            MarketDataType::TrptSubmit
        );
        assert_eq!(
            typed("8=FIX.4.4|35=AE|571=T1|828=1|55=AAPL|32=10|31=5|10=0|"),
            MarketDataType::TrdBlock
        );
        assert_eq!(
            typed("8=FIX.4.4|35=q|11=C1|530=7|10=0|"),
            MarketDataType::McxAll
        );
        assert_eq!(
            typed("8=FIX.4.4|35=V|262=R1|263=1|264=0|10=0|"),
            MarketDataType::MdrSubscribe
        );
        assert_eq!(
            typed("8=FIX.4.4|35=R|131=QR1|303=2|55=AAPL|10=0|"),
            MarketDataType::QrqAutomatic
        );
    }

    /// A dictionary maps any field's values onto the members it chooses
    /// through `FIX:marketdatatype`, before the crate's own reading.
    #[test]
    fn a_registry_maps_its_own_values_onto_a_type() {
        use yggdryl::{FixRegistry, MarketDataType};
        let mut registry = FixRegistry::clone(&super::reader().0);
        let mut ordtype = registry.field(40).expect("OrdType").clone();
        let _: &FixRegistry = &registry;
        ordtype
            .as_fix_mut()
            .set_marketdatatypes(&[("Z", MarketDataType::OrdPegged)])
            .unwrap();
        registry.insert(ordtype).unwrap();
        assert_eq!(
            registry.marketdatatype_of(40, "Z"),
            Some(MarketDataType::OrdPegged)
        );
        assert_eq!(
            registry.marketdatatype_of(40, "2"),
            Some(MarketDataType::OrdLimit)
        );
        let message = super::super::fixed_codec(Arc::new(registry))
            .sole_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=Z|38=5|10=0|")
            .unwrap();
        assert_eq!(message.get_marketdatatype(), MarketDataType::OrdPegged);
    }

    /// A dictionary maps a venue's own `TimeInForce(59)` value onto the
    /// member it stands for; a value it maps nothing for is `OTHER`.
    #[test]
    fn a_registry_maps_its_own_time_in_force_values() {
        use yggdryl::graph::Operation;
        use yggdryl::{FixRegistry, TimeInForce};
        let mut registry = FixRegistry::clone(&super::reader().0);
        let mut tif = registry.field(59).expect("TimeInForce").clone();
        tif.as_fix_mut()
            .set_timeinforces(&[("G", TimeInForce::GoodTillCancel)])
            .unwrap();
        registry.insert(tif).unwrap();
        assert_eq!(
            registry.timeinforce_of(59, "G"),
            Some(TimeInForce::GoodTillCancel)
        );
        assert_eq!(
            registry.timeinforce_of(59, "3"),
            Some(TimeInForce::ImmediateOrCancel)
        );
        assert_eq!(registry.timeinforce_of(59, "Q"), Some(TimeInForce::Other));
        let codec = super::super::fixed_codec(Arc::new(registry));
        let mapped = codec
            .sole_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|59=G|38=5|10=0|")
            .unwrap();
        assert_eq!(mapped.get_timeinforce(), Some(&TimeInForce::GoodTillCancel));
        let standard = codec
            .sole_line(b"8=FIX.4.4|35=D|11=C2|55=AAPL|54=1|59=4|38=5|10=0|")
            .unwrap();
        assert_eq!(standard.get_timeinforce(), Some(&TimeInForce::FillOrKill));
    }

    /// A write moves exactly the facts its field feeds, overwriting: a new
    /// price moves the bid it quoted, a new side the quote and the cross
    /// code, a new order type the type - and a free text none.
    #[test]
    fn a_write_restates_the_facts_its_field_feeds() {
        use yggdryl::graph::Element;
        use yggdryl::{MarketDataType, Side};
        let mut order = parsed("8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=2|44=100|38=5|15=USD|10=0|");
        assert_eq!(order.get_bidpx(), Some(decimal("100")));
        assert_eq!(order.get_crosscode(), "10:1:C1", "an order to buy");

        order.set(44, Scalar::from(decimal("101"))).unwrap();
        assert_eq!(order.get_price(), Some(decimal("101")));
        assert_eq!(order.get_bidpx(), Some(decimal("101")));

        order.set(54, Scalar::from("2")).unwrap();
        assert_eq!(order.get_side(), Side::Sell);
        assert_eq!(
            (order.get_bidpx(), order.get_askpx()),
            (None, Some(decimal("101")))
        );
        assert_eq!(order.get_crosscode(), "10:2:C1", "the side moves the code");

        order.set(40, Scalar::from("1")).unwrap();
        assert_eq!(order.get_marketdatatype(), MarketDataType::OrdMarket);

        // What a caller set through the traits is its word: a write of the
        // field it was read from does not take it back.
        order.set_marketdatatype(MarketDataType::OrdPegged, true);
        order.set(40, Scalar::from("2")).unwrap();
        assert_eq!(order.get_marketdatatype(), MarketDataType::OrdPegged);
        order.set(58, Scalar::from("a note")).unwrap();
        assert_eq!(order.get_price(), Some(decimal("101")));
    }

    #[test]
    fn no_field_fills_the_rates_and_a_settlement_rate_stays_metadata() {
        let stated = parsed(
            "8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|54=1|55=AAPL|31=10|32=1|15=EUR|120=USD|155=1.1|156=D|10=0|",
        );
        assert!(
            stated.get_fxrates().is_empty(),
            "{:?}",
            stated.get_fxrates()
        );
        let metadata = stated.market_data().expect("an execution")[0]
            .get_metadata()
            .clone();
        assert_eq!(
            metadata.get("settlcurrfxrate").map(|held| held.as_str()),
            Some("1.1"),
            "{metadata:?}"
        );
        let fill =
            parsed("8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|54=1|55=EUR/USD|31=1.0862|32=100|10=0|");
        assert!(fill.get_fxrates().is_empty(), "{:?}", fill.get_fxrates());
        // A rate the caller states stands: target to the rate to divide by.
        let mut held = stated;
        let rates = FxRates::from([(Ccy::new("USD").unwrap(), decimal("0.9"))]);
        held.set_fxrates(rates.clone(), true);
        assert_eq!(held.get_fxrates(), &rates);
        assert!(!held.insert_fxrate(Ccy::new("USD").unwrap(), decimal("0.8")));
        assert!(held.insert_fxrate(Ccy::new("GBP").unwrap(), decimal("1.2")));
        assert_eq!(held.get_fxrates().len(), 2);
    }

    #[test]
    fn a_tag_triple_completes_the_member_it_leaves_out() {
        // The last price's spot is `LastSpotRate(194)`, from `LastPx(31)`
        // and `LastForwardPoints(195)` - never `Price(44)`'s.
        let fill = parsed(
            "8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|54=1|15=EUR|44=9|31=1.0862|32=100|195=0.0012|10=0|",
        );
        assert_eq!(fill.get_spotrate(), Some(decimal("1.085")));
        assert_eq!(fill.get_lastpx(), Some(decimal("1.0862")));
        assert_eq!(fill.get_price(), Some(decimal("9")));
        // Three stated are left as they are.
        let three =
            parsed("8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|54=1|31=1|32=100|194=2|195=3|10=0|");
        assert_eq!(
            (
                three.get_lastpx(),
                three.get_spotrate(),
                three.get_forwardpoints()
            ),
            (Some(decimal("1")), Some(decimal("2")), Some(decimal("3")))
        );
    }

    #[test]
    fn an_executions_price_stays_unstated() {
        let mut fill = parsed("8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|54=1|31=101.5|32=10|10=0|");
        for _ in 0..2 {
            assert_eq!(fill.get_price(), None, "a last price is not a price");
            assert_eq!(fill.get_lastpx(), Some(decimal("101.5")));
            assert_eq!(fill.get_lastqty(), Some(decimal("10")));
            // A write settles the message again.
            fill.set(58, Scalar::from("again")).unwrap();
        }
    }

    #[test]
    fn the_category_is_typed_the_rows_word_wins_and_a_new_type_derives_again() {
        let (registry, _) = super::reader();
        let category = |code: &str| {
            registry
                .get_msgtype(code)
                .and_then(|held| held.marketdatakind())
                .unwrap_or(MarketDataKind::Unknown)
        };
        let mut order = parsed("8=FIX.4.4|35=D|11=A|55=AAPL|54=1|38=10|10=0|");
        assert_eq!(order.marketdatakind(), MarketDataKind::Order);
        assert_eq!(
            order.get_by_tag(yggdryl::MARKETDATAKIND_TAG_NAME.0),
            Some(Scalar::MarketDataKind(MarketDataKind::Order))
        );
        // A row stating another category is the row's word.
        let schema = fix_schema(&registry, "fix").unwrap();
        let at = schema
            .index_of("marketdatakind")
            .expect("the category column");
        let mut cells = order
            .into_row(&schema)
            .unwrap()
            .as_sequence()
            .expect("a row")
            .to_vec();
        cells[at] = Scalar::MarketDataKind(MarketDataKind::Book);
        let stated = FixMsg::from_row(
            Arc::clone(&registry),
            &schema,
            &Scalar::from_sequence(cells),
        )
        .unwrap();
        assert_eq!(stated.marketdatakind(), MarketDataKind::Book);
        // A written type derives its own.
        order.set(35, Scalar::from("S")).unwrap();
        assert_eq!(order.marketdatakind(), category("S"));
        assert_ne!(order.marketdatakind(), MarketDataKind::Order);
        // An execution report of no fill is its order's report, whatever
        // category its type files it under.
        order.set(35, Scalar::from("8")).unwrap();
        assert_eq!(category("8"), MarketDataKind::Execution);
        assert_eq!(order.marketdatakind(), MarketDataKind::Order);
        order.set(117, Scalar::from("Q-1")).unwrap();
        assert_eq!(
            order.marketdatakind(),
            MarketDataKind::Quotation,
            "its quote's, naming one"
        );
        order.set(150, Scalar::from("F")).unwrap();
        assert_eq!(
            order.marketdatakind(),
            MarketDataKind::Execution,
            "a fill's report states its fill"
        );
    }

    /// The first leaf's metadata, key by key.
    fn metadata(message: &FixMsg) -> Vec<(String, String)> {
        let leaves = message.market_data().expect("market data");
        leaves[0]
            .get_metadata()
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn value(message: &FixMsg, key: &str) -> Option<String> {
        metadata(message)
            .into_iter()
            .find_map(|(held, value)| (held == key).then_some(value))
    }

    #[test]
    fn a_group_no_column_reads_is_one_json_text_under_its_name() {
        // The parties are the leaf's accounts, so their group writes nothing.
        let order = parsed(
            "8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|38=5|453=2|448=TRADER2|447=D|452=11|448=ACC9|447=D|452=24|10=0|",
        );
        assert_eq!(value(&order, "parties"), None);
        assert!(
            !metadata(&order)
                .iter()
                .any(|(key, _)| key.starts_with("parties")),
            "no member keyed by its path"
        );
        // A decimal is its shortest exact text inside the text, as a string.
        let fees = parsed(
            "8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|54=1|55=AAPL|31=10|32=5|136=1|137=1.5|138=EUR|139=4|10=0|",
        );
        let (key, text) = metadata(&fees)
            .into_iter()
            .find(|(_, value)| value.contains("miscfeeamt"))
            .expect("the fees group");
        assert_eq!(key, "miscfees");
        assert_eq!(
            text,
            r#"[{"miscfeeamt":"1.5","miscfeecurr":"EUR","miscfeetype":"4"}]"#
        );
    }

    #[test]
    fn a_trade_sides_component_is_one_json_text_under_its_bare_name() {
        // A trade's sides are the executions its parse splits off, each
        // carrying its own side's members.
        let sides: Vec<MarketData> = super::super::fixed_codec(super::super::committed_registry())
            .parse_line(
                b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|487=0|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|54=1|1427=BUY-EXEC|1009=4|528=A|54=2|1427=SELL-EXEC|1009=6|528=P|10=0|",
            )
            .expect("a trade")
            .skip(1)
            .map(|side| side.and_then(FixMsg::into_market_data).expect("an execution").remove(0))
            .collect();
        let [buy, sell] = sides.as_slice() else {
            panic!("one execution per side")
        };
        for (side, capacity) in [(buy, "A"), (sell, "P")] {
            assert_eq!(
                side.get_metadata()
                    .get("tradereportorderdetail")
                    .map(ToString::to_string),
                Some(format!(r#"{{"ordercapacity":"{capacity}"}}"#))
            );
        }
    }
}

/// A row states a message's identity by its digests and its place: a cell
/// its column cannot read as the `uint64` it is - a widened column holding
/// a fraction, a text - refuses the row by name rather than reading as
/// zero, because a zero digest is another message's delivery, which the
/// lifecycle folds the message into.
#[test]
fn a_row_stating_a_digest_its_column_cannot_read_is_refused_by_name() {
    let (registry, reader) = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|")
        .unwrap();
    // The fixed row with the content digest widened, as a table with no
    // unsigned type stores it, to a scale that can hold a fraction beside
    // the twenty digits.
    let mut schema = fix_schema(&registry, "fix").unwrap();
    let name = yggdryl::CURRHASHCODE_TAG_NAME.1;
    let widened = DataType::decimal128(22, 2).unwrap().nullable_field(name);
    schema.set_field(name, widened).unwrap();
    let at = schema.index_of(name).expect("a currhashcode column");
    let row = message.into_row(&schema).unwrap();
    // Whole, the digest reads back as the number it was.
    let read = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(read.get_currhashcode(), message.get_currhashcode());
    assert_ne!(read.get_currhashcode(), 0);
    // With a fraction, the row is refused by its column.
    let cells = row.as_sequence().expect("a row");
    let broken = Scalar::from_sequence(cells.iter().enumerate().map(|(index, cell)| {
        if index == at {
            Scalar::decimal128(150, 2)
        } else {
            cell.clone()
        }
    }));
    let error = FixMsg::from_row(Arc::clone(&registry), &schema, &broken)
        .unwrap_err()
        .to_string();
    assert!(error.contains("$.currhashcode"), "{error}");
    assert!(error.contains("uint64"), "{error}");
}

/// A table with no unsigned type may store a digest as the `long` of its
/// width, so a row reads an `int64` digest cell as its bits; the place is a
/// count, read by value alone, so a negative one refuses the row by name.
#[test]
fn a_row_reads_a_long_digest_as_its_bits_and_refuses_a_negative_place() {
    let (registry, reader) = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|")
        .unwrap();
    // The fixed row as a table of longs lays it out, stating nothing.
    let mut schema = fix_schema(&registry, "fix").unwrap();
    for name in [yggdryl::CURRHASHCODE_TAG_NAME.1, yggdryl::SEQNUM_TAG_NAME.1] {
        let mut long = schema.get_field(name).expect("an identity column").clone();
        long.set_dtype(DataType::Int64).unwrap();
        schema.set_field(name, long).unwrap();
    }
    let digest_at = schema
        .index_of(yggdryl::CURRHASHCODE_TAG_NAME.1)
        .expect("a currhashcode column");
    let place_at = schema
        .index_of(yggdryl::SEQNUM_TAG_NAME.1)
        .expect("a seqnum column");
    let fixed = fix_schema(&registry, "fix").unwrap();
    let row = message.into_row(&fixed).unwrap();
    let cells = |digest: Scalar, place: Scalar| {
        Scalar::from_sequence(row.as_sequence().expect("a row").iter().enumerate().map(
            |(index, cell)| match index {
                index if index == digest_at => digest.clone(),
                index if index == place_at => place.clone(),
                _ => cell.clone(),
            },
        ))
    };

    let read = FixMsg::from_row(
        Arc::clone(&registry),
        &schema,
        &cells(Scalar::from(-1_i64), Scalar::from(0_i64)),
    )
    .unwrap();
    assert_eq!(read.get_currhashcode(), u64::MAX);

    let error = FixMsg::from_row(
        Arc::clone(&registry),
        &schema,
        &cells(Scalar::from(-1_i64), Scalar::from(-1_i64)),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("$.seqnum"), "{error}");
}
