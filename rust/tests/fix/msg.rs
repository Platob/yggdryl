//! `rust/src/fix/msg.rs`: the message holder's setters, and the row read back
//! into a message.

use super::SoleMessage;

use std::sync::Arc;

use yggdryl::fix::FIXENTRIES_COLUMN;

use yggdryl::graph::{Element, Event, Market, Operation};
use yggdryl::securityid::{SecType, SecurityId};
use yggdryl::text::{TextBytes, TextLine};
use yggdryl::{
    DataType, Decimal, Field, FixCodec, FixEntry, FixMsg, FixRegistry, Scalar, StructType,
    fix_schema, fix_schema_carrying,
};

/// One security identifier under `key`, validated by its source.
fn securityid(key: &str, code: &str) -> SecurityId {
    SecurityId::new(SecType::read(key).expect("a source"), code).expect("an identifier")
}

fn sectype(key: &str) -> SecType {
    SecType::read(key).expect("a source")
}

fn reader() -> (Arc<FixRegistry>, FixCodec) {
    let registry = super::committed_registry();
    let reader = super::fixed_codec(Arc::clone(&registry));
    (registry, reader)
}

#[test]
fn instrument_identifier_setters_fill_secaltids() {
    let (_, reader) = reader();
    let mut message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A1|454=1|455=AAPL.O|456=5|10=0|")
        .expect("an order with one unrelated alternate identifier");

    for (key, code) in [
        ("ISIN", "US0378331005"),
        ("CUSIP", "037833100"),
        ("SEDOL", "2046251"),
        ("BLOOMBERG", "AAPL US EQUITY"),
        ("FIGI", "BBG000BLNQ16"),
    ] {
        assert!(
            message
                .insert_securityid(securityid(key, code))
                .expect("a key the dictionary has a source field for")
        );
    }

    let alternates = |message: &FixMsg| {
        super::sequence(
            message
                .by_name("secaltids")
                .expect("the alternate identifiers"),
        )
        .into_iter()
        .map(|occurrence| {
            let values = occurrence.as_sequence().expect("an occurrence");
            (
                values[0].as_str().expect("an identifier").to_owned(),
                values[1].as_str().expect("a source").to_owned(),
            )
        })
        .collect::<Vec<_>>()
    };
    assert_eq!(
        alternates(&message),
        [
            ("AAPL.O".to_owned(), "5".to_owned()),
            ("US0378331005".to_owned(), "4".to_owned()),
            ("037833100".to_owned(), "1".to_owned()),
            ("2046251".to_owned(), "2".to_owned()),
            ("AAPL US EQUITY".to_owned(), "A".to_owned()),
            ("BBG000BLNQ16".to_owned(), "S".to_owned()),
        ]
    );
    assert!(message.get_by_name("secaltidgrp").is_none());
    assert_eq!(
        message
            .entries()
            .iter()
            .find(|entry| entry.tag() == 454)
            .and_then(FixEntry::value),
        Some("6")
    );

    // A source's identifier is stated once: inserting under a held source
    // fills nothing, and another value for it is a removal and an
    // insertion, whose occurrence closes the group. Removing another source
    // removes only that one; the unrelated RIC occurrence remains where the
    // input stated it.
    assert!(
        !message
            .insert_securityid(securityid("ISIN", "US5949181045"))
            .unwrap()
    );
    assert!(message.remove_securityid(&sectype("ISIN")).unwrap());
    assert!(
        message
            .insert_securityid(securityid("ISIN", "US5949181045"))
            .unwrap()
    );
    assert!(message.remove_securityid(&sectype("BLOOMBERG")).unwrap());
    assert!(!message.remove_securityid(&sectype("BLOOMBERG")).unwrap());
    assert_eq!(
        alternates(&message),
        [
            ("AAPL.O".to_owned(), "5".to_owned()),
            ("037833100".to_owned(), "1".to_owned()),
            ("2046251".to_owned(), "2".to_owned()),
            ("BBG000BLNQ16".to_owned(), "S".to_owned()),
            ("US5949181045".to_owned(), "4".to_owned()),
        ]
    );
    assert_eq!(message.get_securityids().get("ISIN"), Some("US5949181045"));
    assert_eq!(message.get_securityids().get("BLOOMBERG"), None);

    for key in ["ISIN", "CUSIP", "SEDOL", "FIGI"] {
        assert!(message.remove_securityid(&sectype(key)).unwrap());
    }
    assert_eq!(
        alternates(&message),
        [("AAPL.O".to_owned(), "5".to_owned())]
    );
    assert_eq!(
        message
            .entries()
            .iter()
            .find(|entry| entry.tag() == 454)
            .and_then(FixEntry::value),
        Some("1")
    );
}

/// What the message only derives - the national number its ISIN carries -
/// stays off the wire until it is stated: a stated identifier replaces the
/// derived one and is written, and removing the ISIN takes back only what
/// hung on it.
#[test]
fn a_stated_identifier_replaces_a_derived_one_on_the_wire() {
    let (_, reader) = reader();
    let mut message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A1|48=US0378331005|22=4|10=0|")
        .expect("an order stating its ISIN");
    let on_wire = |message: &FixMsg, code: &str| {
        message.by_name("secaltids").is_ok_and(|group| {
            super::sequence(group).into_iter().any(|occurrence| {
                occurrence.as_sequence().expect("an occurrence")[0].as_str() == Some(code)
            })
        })
    };
    assert_eq!(message.get_securityids().get("CUSIP"), Some("037833100"));
    assert!(!on_wire(&message, "037833100"));

    assert!(
        message
            .insert_securityid(securityid("CUSIP", "037833100"))
            .unwrap()
    );
    assert!(on_wire(&message, "037833100"));
    assert!(
        !message
            .insert_securityid(securityid("CUSIP", "594918104"))
            .unwrap()
    );

    let mut derived = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A1|48=US0378331005|22=4|10=0|")
        .expect("an order stating its ISIN");
    assert!(derived.remove_securityid(&sectype("ISIN")).unwrap());
    assert_eq!(derived.get_securityids().get("CUSIP"), None);
    assert!(message.remove_securityid(&sectype("ISIN")).unwrap());
    assert_eq!(message.get_securityids().get("CUSIP"), Some("037833100"));
}

#[test]
fn crosscode_uses_fix_priority_while_session_events_name_the_observation() {
    let (registry, reader) = reader();
    let message = reader
        .sole_line(
            b"MSGTYPE=8|MSGSEQNUM=7|ORDERID=ORDER-1|CLORDID=CLIENT-1|ORIGCLORDID=CLIENT-0|QUOTEID=QUOTE-1|QUOTEREQID=REQUEST-1|MDREQID=MARKET-1|MSGSESSIONID=SESSION-1|MSGCTXID=CONTEXT-1",
        )
        .unwrap();

    assert_eq!(message.get_crosscode(), "ORDER-1", "FIX priority wins");
    // The names the message goes by are its own; where the bridge delivered
    // it is the capture's word, so the session event is no identifier.
    assert_eq!(
        message.get_altids().iter().collect::<Vec<_>>(),
        [
            ("CLORDID", "CLIENT-1"),
            ("MDREQID", "MARKET-1"),
            ("ORDERID", "ORDER-1"),
            ("ORIGCLORDID", "CLIENT-0"),
            ("QUOTEID", "QUOTE-1"),
            ("QUOTEREQID", "REQUEST-1"),
        ],
        "every identifier a source field states stands, without the capture context"
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
    assert_eq!(fallback.get_crosscode(), "CLIENT-0", "first stated FIX id");

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
    assert!(capture_only.get_altids().is_empty());

    for partial in [
        b"MSGTYPE=ZZ|MSGSEQNUM=7|MSGSESSIONID=SESSION-1".as_slice(),
        b"MSGTYPE=ZZ|MSGSEQNUM=7|MSGCTXID=CONTEXT-1".as_slice(),
        b"MSGTYPE=ZZ|MSGSESSIONID=SESSION-1|MSGCTXID=CONTEXT-1".as_slice(),
    ] {
        let partial = reader.sole_line(partial).unwrap();
        assert_eq!(partial.capture().msgsesseventid(), None);
        assert!(partial.by_tag(yggdryl::MSGSESSEVENTID_TAG_NAME.0).is_err());
        assert!(!partial.get_altids().contains_key("MSGSESSEVENTID"));
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
    assert_eq!(message.get_altids(), other_capture.get_altids());
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
    assert_eq!(rebuilt.get_crosscode(), "ORDER-1");
    assert_eq!(rebuilt.get_altids(), message.get_altids());
    assert_eq!(
        rebuilt.capture().msgsesseventid(),
        Some("8:SESSION-1:CONTEXT-1:7")
    );
    assert_eq!(rebuilt.into_row(&schema).unwrap(), row);
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
    assert_eq!(message.get_altids().get("CLORDID"), Some("CLIENT-1"));
    assert_eq!(message.get_altids().get("ORDERID"), Some("ORDER-1"));

    // A missing part unsays the key rather than leaving a stale one.
    message
        .set(yggdryl::MSGCTXID_TAG_NAME.0, Scalar::Null)
        .unwrap();
    assert_eq!(message.capture().msgsesseventid(), None);
    assert_eq!(message.get_altids().get("CLORDID"), Some("CLIENT-1"));

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
    assert_eq!(
        message.get_crosshashcode(),
        yggdryl::xxhash::xxh3(b"EXPLICIT")
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
    assert_eq!(message.get_crosscode(), "EXPLICIT");
    assert_eq!(
        message.capture().msgsesseventid(),
        Some("F:SESSION-3:CONTEXT-2:8")
    );
    assert!(!message.get_altids().contains_key("MSGSESSEVENTID"));
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
    assert!(!message.get_altids().contains_key("MSGSESSEVENTID"));
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
        assert!(!partial.get_altids().contains_key("MSGSESSEVENTID"));
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
    parsed.set_execunix(Some(200));
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

/// A repeating group holding no occurrence is stated by its count alone:
/// `NoPartySubIDs(802)=0` is the counter stating zero beside the empty
/// list - an entry of its own, which re-emits - and a list holding nothing
/// beside no stated count is the group absent. A table storing a null list
/// as an empty one - PyIceberg reads a null list of structs back as `[]`,
/// the counter beside it still null - so hands each message back as the
/// parse wrote it: settled again from its content, each keeps its content
/// code and its identity, and only the count stated re-emits.
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
    // states, so the fixed row is read without that column.
    let fixed = fix_schema(&registry, "fix").unwrap();
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
    let count_at = party.index_of("nopartysubids").expect("its counter member");
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
        assert_eq!(!members[count_at].is_null(), stated, "the count");
        assert_eq!(!members[subids_at].is_null(), stated, "the subgroup");
        members[subids_at] = Scalar::from_sequence(Vec::<Scalar>::new());
        cells[parties_at] = Scalar::from_sequence([Scalar::from_sequence(members)]);
        let read_back = Scalar::from_sequence(cells);
        assert_eq!(read_back == written, stated, "only the absent list moves");
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
        // Equality compares the row's storage; what the row states agrees.
        assert_eq!(as_read_back == as_written, stated);
        assert_eq!(as_read_back.entries(), as_written.entries());
        assert_eq!(as_read_back.digest(), as_written.digest());
        assert_eq!(wire(&as_read_back), wire(&as_written));
    }
}

/// The same rule at the root of a row: `NoPartyIDs(453)=0` is stated and
/// re-emits, and a `parties` column read back as `[]` where the row held
/// null - its `nopartyids` still null, or no such column at all - is the
/// group absent, so a message settled again from the row keeps its content
/// code and its identity.
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
    // its row states; then without the count's column as well.
    let fixed = fix_schema(&registry, "fix").unwrap();
    let hashcode_at = yggdryl::fix_column_of(&fixed, yggdryl::CURRHASHCODE_TAG_NAME.0)
        .expect("a currhashcode column");
    let count_at = fixed.index_of("nopartyids").expect("a nopartyids column");
    for dropped in [vec![hashcode_at], vec![hashcode_at, count_at]] {
        let schema = StructType::from_fields(
            fixed
                .fields()
                .iter()
                .enumerate()
                .filter(|(at, _)| !dropped.contains(at))
                .map(|(_, column)| column.clone()),
        )
        .map(DataType::from)
        .unwrap()
        .required_field("fix");
        let parties_at = schema.index_of("parties").expect("a parties column");
        for (message, stated) in [(&absent, false), (&counted, true)] {
            let written = message.into_row(&schema).unwrap();
            let mut cells = written.as_sequence().unwrap().to_vec();
            if cells[parties_at].is_null() {
                cells[parties_at] = Scalar::from_sequence(Vec::<Scalar>::new());
            }
            let read_back = Scalar::from_sequence(cells);
            let columns = dropped.len();
            for (row, read) in [(&written, "as written"), (&read_back, "as read back")] {
                let held = FixMsg::from_row(Arc::clone(&registry), &schema, row).unwrap();
                assert_eq!(
                    held.get_currhashcode(),
                    message.get_currhashcode(),
                    "{read}, {columns} column(s) dropped"
                );
                assert_eq!(held.get_curruuid(), message.get_curruuid(), "{read}");
                assert_eq!(wire(&held).contains("|453="), stated, "{read}");
            }
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
        .sole_line(b"8=FIX.4.4|35=D|11=A1|454=1|455=US0378331005|456=4|10=0|")
        .expect("an order stating its ISIN as an alternate identifier");
    let root = parsed.as_field().clone();
    let at = root.index_of("secaltids").expect("the group's column");
    let row = super::with_column_at(parsed.as_value(), at, &super::item_of(&root.fields()[at]));
    let message = FixMsg::with_registry(Arc::clone(&registry), root, row).expect("a message");
    assert!(super::holds_column(&message, "secaltids"));

    assert_eq!(message.get_securityids().get("ISIN"), Some("US0378331005"));
    let group = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 454)
        .expect("the group's entry");
    assert_eq!(group.value(), Some("1"));
    assert_eq!(group.entries().len(), 1);
    assert_eq!(
        message
            .get_by_path(&super::path("secaltids[-1].securityaltid"))
            .as_ref()
            .and_then(Scalar::as_str),
        Some("US0378331005")
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
    assert_eq!(bid.get_side().as_str(), "UNKN");
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

    // A report pricing itself is about the fill: what it last executed at
    // is `lastpx`, never the price.
    let fill = reader
        .sole_line(
            b"8=FIX.4.4|35=8|52=20240102-10:15:30|37=O|17=E|150=F|39=2|31=100|32=10|132=99|10=0|",
        )
        .unwrap();
    assert_eq!(fill.get_side().as_str(), "UNKN");
    assert_eq!((fill.get_price(), fill.get_quantity()), (None, None));
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
    assert_eq!(order.get_side().as_str(), "UNKN");
}

mod market_ladder {
    //! The market a message names, and what a bridge's instrument key fills.

    use yggdryl::FixMsg;
    use yggdryl::graph::Market;

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
            let wire = String::from_utf8(held.into_bytes(b'|')).expect("a text wire");
            assert!(
                wire.contains(&format!("instrument[exchange]={code}|")),
                "the spelling still re-emits: {wire}"
            );
            // A row carries it as its residual entry, and reads it back as
            // the refused spelling rather than the market.
            let registry = super::super::committed_registry();
            let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
            let row = held.into_row(&schema).expect("a row");
            let again =
                FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &row).expect("again");
            assert_eq!(again.get_miccode().map(|held| held.as_str()), Some("XSWX"));
            // By its own child: the name alone reaches the market.
            let at = again
                .as_field()
                .index_of("instrument[exchange]")
                .expect("the refused spelling");
            assert_eq!(
                again.as_value().as_sequence().expect("a row")[at].as_str(),
                Some(code)
            );
        }
    }

    #[test]
    fn the_instrument_key_fills_the_isin_and_the_currency_only_where_open() {
        let held = parsed(&format!(
            "{HEAD}OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|10=0|"
        ));
        assert_eq!(held.get_securityids().get("ISIN"), Some("CH0012214059"));
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
        assert_eq!(held.get_securityids().get("ISIN"), Some("CH0012221716"));
        assert_eq!(held.get_currency().as_str(), "EUR");

        // A part its type refuses is skipped, and the others still answer.
        let held = parsed(&format!(
            "{HEAD}OMSINSTRUMENTID=dbi;CH0012214058_XSWX_CHF|10=0|"
        ));
        assert_eq!(held.get_securityids().get("ISIN"), None);
        assert_eq!(held.get_currency().as_str(), "CHF");
        assert_eq!(held.get_miccode().map(|held| held.as_str()), Some("XSWX"));

        // A value not shaped twelve, four and three names nothing.
        for code in [
            "dbi;CH001221405_XSWX_CHF",
            "dbi;CH0012214059_XSWX",
            "dbi;CH0012214059_XSWX_CHF_X",
            "CH0012214059-XSWX-CHF",
        ] {
            let held = parsed(&format!("{HEAD}OMSINSTRUMENTID={code}|10=0|"));
            assert_eq!(held.get_securityids().get("ISIN"), None, "{code}");
            assert!(held.get_miccode().is_none(), "{code}");
        }
    }
}

mod identifier_maps {
    //! The alternate identifiers a message rebuilds from the dictionary's
    //! `FIX:idmap` sources at every settle, and the accounts its parties and
    //! its `Account(1)` name, which leave its leaves' metadata; a user is no
    //! identifier and stays content.

    use yggdryl::FixMsg;
    use yggdryl::graph::{Market, Operation};

    fn parsed(line: &str) -> FixMsg {
        super::super::fixed_codec(super::super::committed_registry())
            .parse_fix_line(line.as_bytes())
            .expect("a readable line")
    }

    fn pairs(ids: &yggdryl::IdMap) -> Vec<(String, String)> {
        ids.iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    /// The metadata of the first leaf the message expands to: what it
    /// states that no typed column reads.
    fn leaf_metadata(message: &FixMsg) -> yggdryl::graph::Metadata {
        message.market_data().expect("market data")[0]
            .get_metadata()
            .clone()
    }

    #[test]
    fn a_bridges_own_fields_name_the_message_and_stay_content() {
        let held = parsed(
            "8=FIX.4.4|35=8|17=E1|37=O1|150=F|39=2|54=1|55=AAPL|31=10|32=1|1=ACC|\
             OMSDEALERACCOUNT=YNHD5|OMSUSERID=trader1|\
             PARENTORDERID=P1|PARENTCLORDID=PC1|OMSDEALERPARENTORDERID=OP1|\
             EXCHANGECLIENTORDERID=X1|TRANSVERSAL_KEY=T1|ULTRADER_CLORDID=U1|10=0|",
        );
        let alts = pairs(held.get_altids());
        for (key, value) in [
            ("EXCHANGECLIENTORDERID", "X1"),
            ("OMSDEALERPARENTORDERID", "OP1"),
            ("ORDERID", "O1"),
            ("PARENTCLORDID", "PC1"),
            ("PARENTORDERID", "P1"),
            ("TRANSVERSALKEY", "T1"),
            ("ULTRADERCLORDID", "U1"),
            ("EXECID", "E1"),
        ] {
            assert!(
                alts.contains(&(key.to_owned(), value.to_owned())),
                "{key}: {alts:?}"
            );
        }
        // Content, as it arrived, and no column of the fixed row.
        assert_eq!(
            held.get_by_name("transversalkey")
                .and_then(|held| held.as_str().map(str::to_owned))
                .as_deref(),
            Some("T1")
        );
        let registry = super::super::committed_registry();
        let schema = yggdryl::fix_schema(&registry, "fix").expect("a schema");
        for tag in 65_032..=65_039 {
            assert!(yggdryl::fix_column_of(&schema, tag).is_none(), "{tag}");
        }
        // An account and a user are no identifier: no alternate identifier
        // holds them. `Account(1)` is the message's `ACCOUNT` account, which
        // its leaf holds rather than its metadata, and the bridge's own two
        // stay content of the message's row.
        for key in ["ACCOUNT", "OMSDEALERACCOUNT", "OMSUSERID"] {
            assert!(alts.iter().all(|(held, _)| held != key), "{key}: {alts:?}");
        }
        assert_eq!(held.get_accountids().get("ACCOUNT"), Some("ACC"));
        let metadata = leaf_metadata(&held);
        assert!(!metadata.contains_key("account"), "{metadata:?}");
        for (name, value) in [("omsdealeraccount", "YNHD5"), ("omsuserid", "trader1")] {
            assert_eq!(
                held.get_by_name(name)
                    .and_then(|held| held.as_str().map(str::to_owned))
                    .as_deref(),
                Some(value),
                "{name}"
            );
        }
        // A write lands on the field the key is read from.
        let mut held = held;
        assert!(
            held.insert_altid("EXECID", "other")
                .expect("a stated key")
                .eq(&false)
        );
    }

    #[test]
    fn the_parties_a_message_names_are_its_accounts_and_a_second_of_a_role_stays() {
        let held = parsed(
            "8=FIX.4.4|35=8|17=E1|37=O1|150=F|39=2|54=1|55=AAPL|31=10|32=1|453=3|448=T1|\
             447=D|452=36|448=T2|447=D|452=36|\
             448=C1|447=D|452=24|10=0|",
        );
        // The first party of each role is an account; the second trader of
        // one role is ordinary, no anomaly, and it alone stays in the leaf's
        // metadata, since no map the leaf holds names it.
        assert_eq!(
            held.get_accountids().to_string(),
            "{CUSTOMERACCOUNT=C1, ENTERINGTRADER=T1}"
        );
        let metadata = leaf_metadata(&held);
        assert_eq!(
            metadata.get("parties").map(|held| held.as_str()),
            Some(r#"[{"partyid":"T2","partyidsource":"D","partyrole":"36"}]"#)
        );
        // No party is an alternate identifier, so none is an anomaly.
        assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
        assert!(
            pairs(held.get_altids())
                .iter()
                .all(|(key, _)| key == "ORDERID" || key == "EXECID"),
            "{:?}",
            held.get_altids()
        );
    }

    #[test]
    fn a_following_operation_carries_what_the_dictionary_follows() {
        let held = parsed("8=FIX.4.4|35=8|17=E1|37=O1|10=0|");
        for key in [
            "ORDERID",
            "SECONDARYORDERID",
            "PARENTORDERID",
            "PARENTCLORDID",
            "OMSDEALERPARENTORDERID",
            "EXCHANGECLIENTORDERID",
            "TRANSVERSALKEY",
        ] {
            assert!(held.is_followed_altid(key), "{key}");
        }
        for key in ["CLORDID", "EXECID", "ULTRADERCLORDID", "MARKETORDERID"] {
            assert!(!held.is_followed_altid(key), "{key}");
        }
    }
}

/// A message naming no currency pair digests exactly as it did before FX
/// detection existed: detection writes nothing where it finds no pair.
///
/// The pin last moved when an event's place left its content code: the
/// code no longer feeds `seqnum`, which this message states as zero.
#[test]
fn a_message_naming_no_pair_digests_as_it_did_before_detection() {
    let (_, reader) = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|54=1|38=100|40=2|44=10.5|15=USD|167=CS|10=0|")
        .expect("an order");
    assert_eq!(message.get_currhashcode(), 12_856_949_354_363_238_690);
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
        held.set_fxrates(rates.clone());
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
                .and_then(|held| held.msgcat())
                .unwrap_or(MarketDataKind::Unknown)
        };
        let mut order = parsed("8=FIX.4.4|35=D|11=A|55=AAPL|54=1|38=10|10=0|");
        assert_eq!(order.msgcat(), MarketDataKind::Order);
        assert_eq!(
            order.get_by_tag(yggdryl::MSGCAT_TAG_NAME.0),
            Some(Scalar::MarketDataKind(MarketDataKind::Order))
        );
        // A row stating another category is the row's word.
        let schema = fix_schema(&registry, "fix").unwrap();
        let at = schema.index_of("msgcat").expect("the category column");
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
        assert_eq!(stated.msgcat(), MarketDataKind::Book);
        // A written type derives its own.
        order.set(35, Scalar::from("S")).unwrap();
        assert_eq!(order.msgcat(), category("S"));
        assert_ne!(order.msgcat(), MarketDataKind::Order);
        // An execution report of no fill is its order's report, whatever
        // category its type files it under.
        order.set(35, Scalar::from("8")).unwrap();
        assert_eq!(category("8"), MarketDataKind::Execution);
        assert_eq!(order.msgcat(), MarketDataKind::Order);
        order.set(117, Scalar::from("Q-1")).unwrap();
        assert_eq!(
            order.msgcat(),
            MarketDataKind::Quotation,
            "its quote's, naming one"
        );
        order.set(150, Scalar::from("F")).unwrap();
        assert_eq!(
            order.msgcat(),
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
        // A decimal keeps its stored scale inside the text, as a string.
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
            r#"[{"miscfeeamt":"1.500000000000000000","miscfeecurr":"EUR","miscfeetype":"4"}]"#
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
