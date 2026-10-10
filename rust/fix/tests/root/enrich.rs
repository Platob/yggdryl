//! `rust/fix/src/enrich.rs`: the specification's tables read as
//! implications: each one of the crate's native rules, keyed by the tag it
//! fills, answered once from a hand-written line, refused where the answer
//! is not certain, and settled in one parse. A registry carries no rule of
//! its own: every registry fills by the same rules.

use super::SoleMessage;

use std::sync::Arc;

use yggdryl::graph::Event;
use yggdryl::holder::Buffer;
use yggdryl::text::{TextLine, TextOptions, read_text_lines};
use yggdryl::{Figi, Isin, State};
use yggdryl::{Scalar, StringEnum, Timezone, Url};
use yggdryl_fix::FixFieldMut;
use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
use yggdryl_market::graph::{Market, Operation};
use yggdryl_market::{IdKey, IdType};

fn reader() -> FixCodec {
    super::fixed_codec(super::committed_registry())
}

/// One line read, then read again to prove the fill is the read's: a parse
/// states what the message implies, so every chain of rules reaches its end
/// in the one pass and reading the line twice answers one message.
fn settled(reader: &FixCodec, line: &[u8]) -> FixMsg {
    let once = reader.sole_line(line).expect("a readable line");
    let twice = once.clone();
    assert_eq!(
        once,
        twice,
        "a second pass over {}",
        String::from_utf8_lossy(line)
    );
    once
}

/// The text one tag holds, for the assertions that read a spelling.
fn text(message: &FixMsg, tag: i32) -> Option<String> {
    message
        .get_by_tag(tag)
        .as_ref()
        .and_then(Scalar::as_str)
        .map(ToOwned::to_owned)
}

/// The instrument's ISIN, as the trait derives it from `SecurityID(48)`
/// under its source and the `SecurityAltID` group.
fn isincode(message: &FixMsg) -> Option<String> {
    message
        .get_securityids()
        .get(&IdType::Isin)
        .map(ToOwned::to_owned)
}

/// The market, as the trait derives it from `SecurityExchange(207)`,
/// `ExDestination(100)` or `LastMkt(30)`.
fn miccode(message: &FixMsg) -> Option<String> {
    message.get_miccode().map(|held| held.as_str().to_owned())
}

/// The lifecycle state as the trait derives it - off the first status field
/// that states one, else off the message type; `None` where nothing says.
fn statecode(message: &FixMsg) -> Option<String> {
    let held = message.get_state();
    (held != &State::unknown()).then(|| held.as_str().to_owned())
}

/// The integer one tag holds.
fn integer(message: &FixMsg, tag: i32) -> Option<i128> {
    message.get_by_tag(tag).as_ref().and_then(Scalar::as_i128)
}

/// One typed FIX timestamp at nanosecond resolution.
fn instant(message: &FixMsg, tag: i32) -> Option<i64> {
    message
        .get_by_tag(tag)
        .and_then(|held| held.temporal_count_at(yggdryl::TimeUnit::Nanosecond))
}

/// The ranked spelling a `state` column holds for one wire code.
fn state(code: &str) -> String {
    State::from_spelling(code)
        .expect("a lifecycle code")
        .as_str()
        .to_owned()
}

/// A bridge row carrying one alternate identifier under one source.
fn alternate(id: &str, source: &str) -> Vec<u8> {
    format!(
        "MSGTYPE=D|#CLORDID=A|#NOSECURITYALTID=1|#NOSECURITYALTID[0]=SECURITYALTID={id}\x04\x03SECURITYALTIDSOURCE={source}"
    )
    .into_bytes()
}

#[test]
fn smarttrade_quote_and_mass_quote_ack_map_creation_time_and_quote_identifiers() {
    crate::install::installed();
    let reader = reader();
    for (msgtype, canonical) in [("quote", "S"), ("massquoteacknowledgement", "b")] {
        let line = format!(
            "CREATIONTIME=20260814092957|ENV=PROD|MSGTYPE={msgtype}|ORIG.MSGTYPE=massquoteacknowledgement|QUOTEID=quote-20260814-1|QUOTEREQID=request-20260814-1|QUOTESTATUS=canceledduetolockmarket|SYMBOL=EUR/USD|TEXT=MATCHED"
        );
        let message = settled(&reader, line.as_bytes());

        assert_eq!(message.header().msgtype(), canonical);
        assert_eq!(message.get_creaunix(), Some(1_786_699_797_000_000_000));
        assert_eq!(integer(&message, 297), Some(14));
        assert_eq!(text(&message, 55).as_deref(), Some("EUR/USD"));
        assert_eq!(text(&message, 58).as_deref(), Some("MATCHED"));
        assert_eq!(
            message
                .get_identifiers()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["quoteid=quote-20260814-1", "quotereqid=request-20260814-1",]
        );
    }
}

#[test]
fn smarttrade_ulbridge_rows_keep_quote_and_mass_quote_ack_as_two_deliveries() {
    crate::install::installed();
    let rows = concat!(
        "2026-08-14 11:29:57.511 [636-e7254b17:9f02625007:86416] [SmartTrade_RFQ] (DEBUG) RouteMessage : ",
        "MSGTYPE=massquoteacknowledgement|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=LUX|\n",
        "2026-08-14 11:29:57.511 [636-e7254b17:9f02625007:86416] [SmartTrade_RFQ_Add_Fields] (DEBUG) After Enrichment -> ",
        "CREATIONTIME=20260814092957|ENV=PROD|MSGTYPE=quote|ORIG.MSGTYPE=massquoteacknowledgement|",
        "QUOTEID=quote-20260814-1|QUOTEREQID=request-20260814-1|",
        "QUOTESTATUS=canceledduetolockmarket|SYMBOL=EUR/USD|TEXT=MATCHED|\n",
    );
    let source = Buffer::from_bytes(rows.as_bytes().to_vec()).with_media_type(
        Url::from_str("file:///smarttrade.log")
            .expect("a URL")
            .media_type(),
    );
    let options = TextOptions::new()
        .try_with_rowheader(yggdryl_fix::ULBRIDGE_ROWHEADER)
        .expect("the bridge row header compiles")
        .with_timezone(Timezone::UTC);
    let reader = reader().with_capture_names(options.capture_names().map(ToOwned::to_owned));
    let parsed: Vec<FixMsg> = reader
        .parse_text_lines(read_text_lines(&source, &options).expect("a line reader"))
        .collect::<yggdryl::Result<_>>()
        .expect("two readable messages");
    assert_eq!(parsed.len(), 2);
    assert_eq!(
        parsed
            .iter()
            .map(|message| message.header().msgtype())
            .collect::<Vec<_>>(),
        ["b", "S"]
    );

    let quote = &parsed[1];
    assert_eq!(quote.header().msgseqnum(), Some(86_416));
    assert_eq!(quote.capture().msgsessionid(), Some("e7254b17"));
    assert_eq!(quote.capture().msgctxid(), Some("9f02625007"));
    assert_eq!(quote.get_creaunix(), Some(1_786_699_797_000_000_000));
    assert_eq!(
        quote
            .get_identifiers()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["quoteid=quote-20260814-1", "quotereqid=request-20260814-1",]
    );
    // The session event it was delivered as is the capture's, its four
    // values joined as stated.
    assert_eq!(
        quote.capture().msgsesseventid(),
        Some("S:e7254b17:9f02625007:86416")
    );
    assert_eq!(
        parsed[0].capture().msgsesseventid(),
        Some("b:e7254b17:9f02625007:86416")
    );

    let walked = reader
        .lifecycle(parsed)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("a readable lifecycle");
    assert_eq!(
        walked
            .iter()
            .map(|message| message.header().msgtype())
            .collect::<Vec<_>>(),
        ["b", "S"],
        "one capture session/context/sequence under two message types is two deliveries"
    );
}

#[test]
fn execution_time_uses_the_first_execution_specific_statement() {
    crate::install::installed();
    const DIRECT: i64 = 1_704_190_530_100_000_000;
    const EXECUTION: i64 = 1_704_190_530_200_000_000;
    const REGULATORY: i64 = 1_704_190_530_300_000_000;
    const EVENT: i64 = 1_704_190_530_400_000_000;
    const TRANSACTION: i64 = 1_704_190_530_500_000_000;

    let reader = reader();
    let direct = settled(
        &reader,
        b"8=FIX.4.4|35=8|65024=20240102-10:15:30.100|2749=20240102-10:15:30.200|768=2|769=20240102-10:15:30.250|770=2|769=20240102-10:15:30.300|770=1|eventtimestamp=20240102-10:15:30.400|150=F|60=20240102-10:15:30.500|10=0|",
    );
    assert_eq!(direct.get_execunix(), Some(DIRECT));

    let execution = settled(
        &reader,
        b"8=FIX.4.4|35=8|2749=20240102-10:15:30.200|768=1|769=20240102-10:15:30.300|770=1|eventtimestamp=20240102-10:15:30.400|150=F|60=20240102-10:15:30.500|10=0|",
    );
    assert_eq!(execution.get_execunix(), Some(EXECUTION));
    assert_eq!(instant(&execution, 2749), Some(EXECUTION));

    let regulatory = settled(
        &reader,
        b"8=FIX.4.4|35=8|768=2|769=20240102-10:15:30.250|770=2|769=20240102-10:15:30.300|770=1|eventtimestamp=20240102-10:15:30.400|150=F|60=20240102-10:15:30.500|10=0|",
    );
    assert_eq!(regulatory.get_execunix(), Some(REGULATORY));

    let event = settled(
        &reader,
        b"MSGTYPE=8|eventtimestamp=20240102-10:15:30.400|EXECTYPE=F|TRANSACTTIME=20240102-10:15:30.500",
    );
    assert_eq!(event.get_execunix(), Some(EVENT));

    let transaction = settled(
        &reader,
        b"8=FIX.4.4|35=8|150=F|60=20240102-10:15:30.500|10=0|",
    );
    assert_eq!(transaction.get_execunix(), Some(TRANSACTION));
    for execution_type in ["F", "1", "2"] {
        let line = format!("8=FIX.4.4|35=8|150={execution_type}|60=20240102-10:15:30.500|10=0|");
        let message = settled(&reader, line.as_bytes());
        assert_eq!(
            message.get_execunix(),
            Some(TRANSACTION),
            "{execution_type}"
        );
        // The report is its order's: the execution is the message its parse
        // splits off, which dates its fill the same way (A12).
        assert!(!message.is_execution(), "ExecType {execution_type}");
        let split: Vec<FixMsg> = reader
            .parse_line(line.as_bytes())
            .expect("a readable line")
            .collect::<yggdryl::Result<_>>()
            .expect("every message");
        assert!(split[1].is_execution(), "ExecType {execution_type}");
        assert_eq!(
            split[1].get_execunix(),
            Some(TRANSACTION),
            "{execution_type}"
        );
        let walked = reader
            .lifecycle([message])
            .next()
            .expect("one walked message")
            .expect("a readable lifecycle");
        assert_eq!(walked.get_execunix(), Some(TRANSACTION), "{execution_type}");
    }
}

#[test]
fn corrections_cancels_and_clearing_transitions_do_not_invent_execution_time() {
    crate::install::installed();
    let reader = reader();
    for (execution_type, order_status) in [("G", "1"), ("H", "2"), ("J", "1"), ("K", "2")] {
        let line = format!(
            "8=FIX.4.4|35=8|39={order_status}|150={execution_type}|60=20240102-10:15:30.500|10=0|"
        );
        let message = settled(&reader, line.as_bytes());
        assert_eq!(
            message.get_execunix(),
            None,
            "ExecType {execution_type} with OrdStatus {order_status}"
        );
        assert!(
            !message.is_execution(),
            "ExecType {execution_type} is not an execution"
        );
        let walked = reader
            .lifecycle([message])
            .next()
            .expect("one walked message")
            .expect("a readable lifecycle");
        assert_eq!(
            walked.get_execunix(),
            None,
            "walking ExecType {execution_type} must not use current time"
        );
    }
}

#[test]
fn an_identifier_names_the_standard_that_closes_it() {
    crate::install::installed();
    let reader = reader();
    // ISO 6166 closes a number with a check digit, and so do CUSIP and
    // SEDOL: a `SecurityID` one of them closes has stated its own source.
    let isin = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=US0378331005|10=0|");
    assert_eq!(text(&isin, 22).as_deref(), Some("4"));
    assert_eq!(isincode(&isin).as_deref(), Some("US0378331005"));
    assert_eq!(
        text(&isin, 470).as_deref(),
        Some("US"),
        "the country the prefix names"
    );

    let cusip = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=037833100|10=0|");
    assert_eq!(text(&cusip, 22).as_deref(), Some("1"));
    assert_eq!(isincode(&cusip), None);
    assert_eq!(
        cusip.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    assert_eq!(cusip.get_by_tag(470), None);

    let sedol = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=B0YBKJ7|10=0|");
    assert_eq!(text(&sedol, 22).as_deref(), Some("2"));
    assert_eq!(sedol.get_securityids().get(&IdType::Sedol), Some("B0YBKJ7"));

    // Case does not change what a check digit closes.
    let folded = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=us0378331005|10=0|");
    assert_eq!(text(&folded, 22).as_deref(), Some("4"));
    assert_eq!(isincode(&folded).as_deref(), Some("US0378331005"));

    // A value no standard closes answers nothing, and a typo is not an
    // identifier of anything.
    for line in [
        &b"8=FIX.4.4|35=D|11=A|48=HIGH_TOUCH|10=0|"[..],
        b"8=FIX.4.4|35=D|11=A|48=US0378331006|10=0|",
        b"8=FIX.4.4|35=D|11=A|48=037833101|10=0|",
        b"8=FIX.4.4|35=D|11=A|48=B0YBKJ8|10=0|",
    ] {
        let held = settled(&reader, line);
        assert_eq!(
            held.get_by_tag(22),
            None,
            "{}",
            String::from_utf8_lossy(line)
        );
        assert_eq!(isincode(&held), None);
    }

    // A stated source wins over what the value would validate as, and an
    // ISIN under another source is not read as one.
    let stated = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=US0378331005|22=1|10=0|");
    assert_eq!(text(&stated, 22).as_deref(), Some("1"));
    assert_eq!(isincode(&stated), None);

    // A bridge row spelling the source in its own word has stated one,
    // which stands on the wire as spelled; the security identifiers read
    // the source through `SecType::read`, which knows the name as well as
    // the code, so the set names the ISIN the row stated.
    let worded = settled(
        &reader,
        b"MSGTYPE=D|CLORDID=A|SECURITYID=CH0012221716|SECURITYIDSOURCE=isin",
    );
    assert_eq!(text(&worded, 22).as_deref(), Some("isin"));
    assert_eq!(isincode(&worded).as_deref(), Some("CH0012221716"));
}

#[test]
fn a_figi_in_securityid_states_its_own_source_and_never_an_isin() {
    crate::install::installed();
    let reader = reader();
    // Microsoft's composite FIGI closes ISO 6166's digit too; a twelve-byte
    // code behind `BBG` is a FIGI, as a symbol's shape reads it.
    assert!(Isin::new("BBG000BPH459").is_ok());
    let figi = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|55=MSFT|48=BBG000BPH459|10=0|",
    );
    assert_eq!(text(&figi, 22).as_deref(), Some("S"));
    assert_eq!(isincode(&figi), None, "{}", figi.get_securityids());
    assert_eq!(
        figi.get_securityids().get_from(&IdKey::base(IdType::Figi)),
        Some("BBG000BPH459")
    );
    assert_eq!(figi.get_by_tag(470), None, "a FIGI names no country");
    // A `BBG` code no FIGI check closes answers nothing.
    let typo = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|55=MSFT|48=BBG000BPH458|10=0|",
    );
    assert_eq!(typo.get_by_tag(22), None);
}

#[test]
fn an_isin_behind_bbg_that_no_figi_check_closes_states_an_isins_source() {
    crate::install::installed();
    let reader = reader();
    // Barbados is `BB`, so its ISINs may open `BBG`: a code no FIGI's check
    // closes is the ISIN ISO 6166's digit closes.
    assert!(Isin::new("BBGA00000004").is_ok());
    assert!(Figi::new("BBGA00000004").is_err());
    let barbados = settled(&reader, b"8=FIX.4.4|35=D|11=A|55=X|48=BBGA00000004|10=0|");
    assert_eq!(text(&barbados, 22).as_deref(), Some("4"));
    assert_eq!(isincode(&barbados).as_deref(), Some("BBGA00000004"));
    assert_eq!(text(&barbados, 470).as_deref(), Some("BB"));
    // A `SecurityID` filled from an ISIN alternate is an ISIN whatever its
    // shape: the alternate's source names it, so a code a FIGI's check
    // closes too is still the ISIN it was stated as.
    let alternate = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|55=MSFT|454=1|455=BBG000BPH459|456=4|10=0|",
    );
    assert_eq!(text(&alternate, 48).as_deref(), Some("BBG000BPH459"));
    assert_eq!(text(&alternate, 22).as_deref(), Some("4"));
    assert_eq!(
        alternate.get_securityids().to_string(),
        "[isin=BBG000BPH459]"
    );
    // So is a `SecurityID` the message states that is the first ISIN
    // alternate's code: the rule reads the code, not who put it there.
    let stated = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|55=MSFT|48=BBG000BPH459|454=1|455=BBG000BPH459|456=4|10=0|",
    );
    assert_eq!(text(&stated, 22).as_deref(), Some("4"));
    assert_eq!(stated.get_securityids().to_string(), "[isin=BBG000BPH459]");
}

#[test]
fn an_isin_reaches_its_normalized_column_from_wherever_the_message_put_it() {
    crate::install::installed();
    let reader = reader();
    // The alternate identifier whose source says ISIN is the ISIN, and once
    // the row holds one it holds the primary identifier and its source too.
    let held = settled(&reader, &alternate("CH0012221716", "4"));
    assert_eq!(isincode(&held).as_deref(), Some("CH0012221716"));
    assert_eq!(text(&held, 48).as_deref(), Some("CH0012221716"));
    assert_eq!(text(&held, 22).as_deref(), Some("4"));
    assert_eq!(text(&held, 470).as_deref(), Some("CH"));

    // A message stating an ISIN in both places states it in `SecurityID`,
    // as the column is defined: the alternate answers only where the primary
    // did not, and the country is the primary's.
    let both = settled(
        &reader,
        b"MSGTYPE=D|#CLORDID=A|#SECURITYID=US0378331005|#NOSECURITYALTID=1|#NOSECURITYALTID[0]=SECURITYALTID=CH0012221716\x04\x03SECURITYALTIDSOURCE=4",
    );
    assert_eq!(text(&both, 22).as_deref(), Some("4"));
    assert_eq!(isincode(&both).as_deref(), Some("US0378331005"));
    assert_eq!(text(&both, 470).as_deref(), Some("US"));
    // Under another source the primary is no ISIN, and the alternate is.
    let sourced = settled(
        &reader,
        b"MSGTYPE=D|#CLORDID=A|#SECURITYID=037833100|#NOSECURITYALTID=1|#NOSECURITYALTID[0]=SECURITYALTID=CH0012221716\x04\x03SECURITYALTIDSOURCE=4",
    );
    assert_eq!(text(&sourced, 22).as_deref(), Some("1"));
    assert_eq!(text(&sourced, 48).as_deref(), Some("037833100"));
    assert_eq!(isincode(&sourced).as_deref(), Some("CH0012221716"));
    assert_eq!(text(&sourced, 470).as_deref(), Some("CH"));

    // A bridge row stating an ISIN under FIX's own name has stated the
    // primary identifier, and the source and the country follow from it.
    let bridge = settled(&reader, b"MSGTYPE=D|CLORDID=A|SECURITYID=GB0002634946");
    assert_eq!(text(&bridge, 48).as_deref(), Some("GB0002634946"));
    assert_eq!(text(&bridge, 22).as_deref(), Some("4"));
    assert_eq!(text(&bridge, 470).as_deref(), Some("GB"));
    assert_eq!(isincode(&bridge).as_deref(), Some("GB0002634946"));
    // A bridge row spelling `ISINCODE` writes the crate's normalized code
    // column directly. It does not synthesize the separate `SecurityID(48)`
    // message child or its source.
    let spelled = settled(&reader, b"MSGTYPE=D|CLORDID=A|ISINCODE=GB0002634946");
    assert_eq!(isincode(&spelled).as_deref(), Some("GB0002634946"));
    assert_eq!(spelled.get_by_tag(48), None);
    assert_eq!(
        spelled
            .by_name("isincode")
            .expect("the normalized code column")
            .as_str(),
        Some("GB0002634946")
    );

    // An international prefix is an agency and not a country.
    for id in ["XS0000000009", "EU0000000008"] {
        let held = settled(&reader, &alternate(id, "4"));
        assert_eq!(isincode(&held).as_deref(), Some(id));
        assert_eq!(held.get_by_tag(470), None, "{id}");
    }

    // An alternate identifier under another source is not an ISIN; one the
    // check digit does not close is a statement of the number it is - rank
    // zero, which fills `SecurityID` and its source as any alternate does
    // and names no country.
    let cusip = settled(&reader, &alternate("037833100", "1"));
    assert_eq!(isincode(&cusip), None);
    assert_eq!(
        cusip.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    assert_eq!(cusip.get_by_tag(48), None);
    let masked = settled(&reader, &alternate("XX0000000001", "4"));
    assert_eq!(isincode(&masked).as_deref(), Some("XX0000000001"));
    assert_eq!(text(&masked, 48).as_deref(), Some("XX0000000001"));
    assert_eq!(text(&masked, 22).as_deref(), Some("4"));
    assert_eq!(masked.get_by_tag(470), None);
}

#[test]
fn an_alternate_isin_fills_securityid_only_under_no_source_or_an_isins() {
    crate::install::installed();
    let reader = reader();
    // An exchange symbol's source states what `SecurityID` would be: an
    // ISIN is none, so it stays the alternate it was and no symbol is read
    // off it.
    let symbol = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|22=8|454=1|455=US0378331005|456=4|10=0|",
    );
    assert_eq!(symbol.get_by_tag(48), None);
    assert_eq!(symbol.get_by_tag(55), None);
    assert_eq!(
        symbol.get_securityids().to_string(),
        "[cusip=037833100, derived:cusip=037833100, isin=US0378331005]"
    );
    // Under a CUSIP's source no `SecurityID` is made for the source to
    // refuse.
    let cusip = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|55=AAPL|22=1|454=1|455=US0378331005|456=4|10=0|",
    );
    assert_eq!(cusip.get_by_tag(48), None);
    assert!(cusip.anomalies().is_empty(), "{:?}", cusip.anomalies());
    assert_eq!(isincode(&cusip).as_deref(), Some("US0378331005"));
    // Under an ISIN's own source the alternate is the `SecurityID`.
    let isin = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|55=AAPL|22=4|454=1|455=US0378331005|456=4|10=0|",
    );
    assert_eq!(text(&isin, 48).as_deref(), Some("US0378331005"));
}

#[test]
fn a_symbol_is_what_an_exchange_or_bloomberg_called_the_instrument() {
    crate::install::installed();
    let reader = reader();
    let exchange = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=ABBN|22=8|10=0|");
    assert_eq!(text(&exchange, 55).as_deref(), Some("ABBN"));
    let bloomberg = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=ABBN SW|22=A|10=0|");
    assert_eq!(text(&bloomberg, 55).as_deref(), Some("ABBN SW"));

    // Under an ISIN the primary identifier is no symbol, but an exchange's
    // alternate identifier is.
    let alternate = settled(
        &reader,
        b"MSGTYPE=D|#CLORDID=A|#SECURITYID=CH0012221716|#SECURITYIDSOURCE=4|#NOSECURITYALTID=1|#NOSECURITYALTID[0]=SECURITYALTID=ABBN\x04\x03SECURITYALTIDSOURCE=8",
    );
    assert_eq!(text(&alternate, 55).as_deref(), Some("ABBN"));
    let isin = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=CH0012221716|22=4|10=0|");
    assert_eq!(isin.get_by_tag(55), None);

    let stated = settled(&reader, b"8=FIX.4.4|35=D|11=A|55=NOVN|48=ABBN|22=8|10=0|");
    assert_eq!(text(&stated, 55).as_deref(), Some("NOVN"));
}

#[test]
fn a_symbol_a_rule_fills_names_its_currency_pair_as_a_stated_one_does() {
    crate::install::installed();
    let reader = reader();
    // Detected before any other rule reads the cells detection fills, as a
    // stated symbol is: a currency the message states is one leg of the
    // pair, never copied onto the other.
    for currency in ["", "15=EUR|", "120=USD|"] {
        let stated = settled(
            &reader,
            format!(
                "8=FIX.4.4|35=D|11=A|55=EUR/USD|22=8|48=EUR/USD|{currency}54=1|38=1000000|40=1|10=0|"
            )
            .as_bytes(),
        );
        let filled = settled(
            &reader,
            format!("8=FIX.4.4|35=D|11=A|22=8|48=EUR/USD|{currency}54=1|38=1000000|40=1|10=0|")
                .as_bytes(),
        );
        assert_eq!(text(&filled, 55).as_deref(), Some("EUR/USD"), "{currency}");
        // The FX pair derived its minted ISIN, D42: the number of its key
        // `IF:EUR/USD`, beside the pair.
        assert_eq!(
            filled.get_securityids().to_string(),
            "[derived:forex=EUR/USD, derived:isin=QYLTVIRYHNX5, exchsymb=EUR/USD, forex=EUR/USD, isin=QYLTVIRYHNX5]",
            "{currency}"
        );
        assert_eq!(filled.get_instcode(), Some("IF:EUR/USD"), "{currency}");
        assert_eq!(filled.get_securityids(), stated.get_securityids());
        for tag in [15, 120, 167, 460, 461, 2897] {
            assert_eq!(
                filled.get_by_tag(tag),
                stated.get_by_tag(tag),
                "{currency}{tag}"
            );
        }
        assert_eq!(text(&filled, 15).as_deref(), Some("EUR"), "{currency}");
        assert_eq!(miccode(&filled).as_deref(), Some("XXXX"));
    }
    for currency in ["15=EUR|", "120=USD|"] {
        let filled = settled(
            &reader,
            format!("8=FIX.4.4|35=D|11=A|22=8|48=EUR/USD|{currency}54=1|38=1000000|40=1|10=0|")
                .as_bytes(),
        );
        assert_eq!(text(&filled, 120).as_deref(), Some("USD"), "{currency}");
    }
}

#[test]
fn a_cfi_and_a_security_type_state_each_other() {
    crate::install::installed();
    let reader = reader();
    // Appendix 6-D at its category level: the equity category is common
    // stock, and the dictionary files common stock under the equity product.
    let share = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=ESVTFR|10=0|");
    assert_eq!(text(&share, 167).as_deref(), Some("CS"));
    assert_eq!(integer(&share, 460), Some(5));
    let folded = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=esvtfr|10=0|");
    assert_eq!(text(&folded, 167).as_deref(), Some("CS"));

    // An option's second character is its exercise, and an option on a
    // future is its own security type; neither is a product of its own.
    let call = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=OCFXXX|10=0|");
    assert_eq!(text(&call, 167).as_deref(), Some("OOF"));
    assert_eq!(integer(&call, 201), Some(1));
    assert_eq!(call.get_by_tag(460), None);
    let put = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=OPEICS|10=0|");
    assert_eq!(text(&put, 167).as_deref(), Some("OPT"));
    assert_eq!(integer(&put, 201), Some(0));
    let open = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=HXXXXX|10=0|");
    assert_eq!(text(&open, 167).as_deref(), Some("OPT"));
    assert_eq!(
        open.get_by_tag(201),
        None,
        "an exercise the code leaves open"
    );

    // A financing's category is one product, and its group is one type.
    let repo = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=LRXXXX|10=0|");
    assert_eq!(text(&repo, 167).as_deref(), Some("REPO"));
    assert_eq!(integer(&repo, 460), Some(13));

    // A plain bond's category is shared by every kind of bond, so no one
    // security type is certain of it.
    let bond = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=DBFUFR|10=0|");
    assert_eq!(bond.get_by_tag(167), None);
    assert_eq!(bond.get_by_tag(460), None);
    assert_eq!(bond.get_by_tag(201), None);

    // The other way: a security type states its category and group, an
    // option's exercise comes from `PutOrCall`, and the product from the
    // group the dictionary files the type under.
    for (line, cfi, product) in [
        (&b"8=FIX.4.4|35=D|11=A|167=CS|10=0|"[..], "ESXXXX", Some(5)),
        (b"8=FIX.4.4|35=D|11=A|167=OPT|201=0|10=0|", "OPXXXX", None),
        (b"8=FIX.4.4|35=D|11=A|167=OPT|201=1|10=0|", "OCXXXX", None),
        (b"8=FIX.4.4|35=D|11=A|167=OPT|10=0|", "OXXXXX", None),
        (b"8=FIX.4.4|35=D|11=A|167=OOF|201=1|10=0|", "OCFXXX", None),
        (b"8=FIX.4.4|35=D|11=A|167=CORP|10=0|", "DBXXXX", Some(3)),
        (b"8=FIX.4.4|35=D|11=A|167=FRN|10=0|", "DBVXXX", Some(3)),
        (b"8=FIX.4.4|35=D|11=A|167=TBILL|10=0|", "DYXXXX", Some(6)),
        (b"8=FIX.4.4|35=D|11=A|167=GO|10=0|", "DNXXXX", Some(11)),
        (b"8=FIX.4.4|35=D|11=A|167=FUT|10=0|", "FXXXXX", None),
        (b"8=FIX.4.4|35=D|11=A|167=ETF|10=0|", "CEXXXX", None),
        (b"8=FIX.4.4|35=D|11=A|167=FXSPOT|10=0|", "IFXXXP", Some(4)),
    ] {
        let held = settled(&reader, line);
        let spelled = String::from_utf8_lossy(line);
        assert_eq!(text(&held, 461).as_deref(), Some(cfi), "{spelled}");
        assert_eq!(integer(&held, 460), product, "{spelled}");
    }
    // An option stating no exercise leaves it open rather than stating one
    // back off the code it was given.
    let open = settled(&reader, b"8=FIX.4.4|35=D|11=A|167=OPT|10=0|");
    assert_eq!(open.get_by_tag(201), None);

    // A type the specification does not list answers nothing.
    let unknown = settled(&reader, b"8=FIX.4.4|35=D|11=A|167=NOSUCH|10=0|");
    assert_eq!(unknown.get_by_tag(461), None);
    assert_eq!(unknown.get_by_tag(460), None);

    // Stated values stand, whatever the other would imply.
    let stated = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|461=ESVTFR|167=PS|460=12|10=0|",
    );
    assert_eq!(text(&stated, 167).as_deref(), Some("PS"));
    assert_eq!(integer(&stated, 460), Some(12));
}

#[test]
fn the_crates_market_and_state_columns_are_stated_on_the_message() {
    crate::install::installed();
    let reader = reader();
    // Where it last traded, then where it was routed, then where it is
    // listed.
    for (line, market) in [
        (
            &b"8=FIX.4.4|35=D|11=A|207=XSWX|100=XNAS|30=XLON|10=0|"[..],
            "XLON",
        ),
        (b"8=FIX.4.4|35=D|11=A|207=XSWX|100=XNAS|10=0|", "XNAS"),
        (b"8=FIX.4.4|35=D|11=A|207=XSWX|10=0|", "XSWX"),
    ] {
        let held = settled(&reader, line);
        assert_eq!(miccode(&held).as_deref(), Some(market));
    }
    let silent = settled(&reader, b"8=FIX.4.4|35=D|11=A|10=0|");
    assert_eq!(miccode(&silent), None);

    // The column spells a state by its rank, whichever code stated it.
    let status = settled(&reader, b"8=FIX.4.4|35=8|39=1|150=F|10=0|");
    assert_eq!(statecode(&status).as_deref(), Some(state("1").as_str()));
    let trade = settled(&reader, b"8=FIX.4.4|35=8|150=F|10=0|");
    assert_eq!(statecode(&trade).as_deref(), Some(state("F").as_str()));
    assert_eq!(trade.get_by_tag(39), None, "a trade alone says no status");
}

#[test]
fn a_currency_states_the_one_a_trade_settles_in_and_back() {
    crate::install::installed();
    let reader = reader();
    let settling = settled(&reader, b"8=FIX.4.4|35=8|120=USD|10=0|");
    assert_eq!(text(&settling, 15).as_deref(), Some("USD"));
    let dealt = settled(&reader, b"8=FIX.4.4|35=8|15=EUR|10=0|");
    assert_eq!(text(&dealt, 120).as_deref(), Some("EUR"));
    let both = settled(&reader, b"8=FIX.4.4|35=8|15=EUR|120=USD|10=0|");
    assert_eq!(text(&both, 15).as_deref(), Some("EUR"));
    assert_eq!(text(&both, 120).as_deref(), Some("USD"));
}

#[test]
fn an_order_stating_no_time_in_force_is_a_day_order() {
    crate::install::installed();
    let reader = reader();
    for line in [
        &b"8=FIX.4.4|35=D|11=A|10=0|"[..],
        b"8=FIX.4.4|35=G|11=B|41=A|10=0|",
        b"8=FIX.4.4|35=8|37=A|10=0|",
    ] {
        let held = settled(&reader, line);
        assert_eq!(
            text(&held, 59).as_deref(),
            Some("0"),
            "{}",
            String::from_utf8_lossy(line)
        );
    }
    // A quote and a cancel request carry no time in force to default.
    let quote = settled(&reader, b"8=FIX.4.4|35=S|117=Q1|10=0|");
    assert_eq!(quote.get_by_tag(59), None);
    let cancel = settled(&reader, b"8=FIX.4.4|35=F|11=B|41=A|10=0|");
    assert_eq!(cancel.get_by_tag(59), None);
    let stated = settled(&reader, b"8=FIX.4.4|35=D|11=A|59=1|10=0|");
    assert_eq!(text(&stated, 59).as_deref(), Some("1"));
}

#[test]
fn a_report_states_its_status_where_its_execution_type_or_its_quantities_do() {
    crate::install::installed();
    let reader = reader();
    // The values the two code sets spell alike.
    let new = settled(&reader, b"8=FIX.4.4|35=8|150=0|10=0|");
    assert_eq!(text(&new, 39).as_deref(), Some("0"));
    let pending = settled(&reader, b"8=FIX.4.4|35=8|150=A|10=0|");
    assert_eq!(text(&pending, 39).as_deref(), Some("A"));
    // `D` is Restated in one and AcceptedForBidding in the other.
    let restated = settled(&reader, b"8=FIX.4.4|35=8|150=D|10=0|");
    assert_eq!(restated.get_by_tag(39), None);

    // A trade says what happened, and the quantities say where that leaves
    // the order: nothing left is filled, something left and something done
    // is partially filled. A `FILLED` a trade's `LeavesQty(151)` of nothing
    // reads - derived, or stated beside it - is the venue's remainder,
    // which a lifecycle walk's count may contradict (P12), never its own
    // word; a live status is.
    use yggdryl::State;
    use yggdryl::graph::Event;
    use yggdryl_market::graph::Operation;
    let filled = settled(&reader, b"8=FIX.4.4|35=8|150=F|151=0|14=100|10=0|");
    assert_eq!(text(&filled, 39).as_deref(), Some("2"));
    assert!(!filled.states_end(), "the remainder's reading");
    let corrected = settled(&reader, b"8=FIX.4.4|35=8|150=G|151=0|10=0|");
    assert_eq!(text(&corrected, 39).as_deref(), Some("2"));
    let partial = settled(&reader, b"8=FIX.4.4|35=8|150=F|151=60|14=40|10=0|");
    assert_eq!(text(&partial, 39).as_deref(), Some("1"));
    assert!(partial.states_end(), "a live state is its own word");
    for line in [
        &b"8=FIX.4.4|35=8|150=F|10=0|"[..],
        b"8=FIX.4.4|35=8|150=F|151=60|10=0|",
        b"8=FIX.4.4|35=8|150=F|151=60|14=0|",
        b"8=FIX.4.4|35=8|150=F|14=40|10=0|",
    ] {
        let held = settled(&reader, line);
        assert_eq!(
            held.get_by_tag(39),
            None,
            "{}",
            String::from_utf8_lossy(line)
        );
        assert_eq!(*held.get_state(), State::Trade);
    }
    let stated_filled = settled(&reader, b"8=FIX.4.4|35=8|39=2|150=F|151=0|14=100|10=0|");
    assert!(!stated_filled.states_end(), "the same remainder, stated");
    let filled_left = settled(&reader, b"8=FIX.4.4|35=8|39=2|150=F|151=10|14=90|10=0|");
    assert_eq!(*filled_left.get_state(), State::Filled);
    assert!(
        filled_left.states_end(),
        "a filled status beside a remainder"
    );

    // Only a report speaks for an order's status, and a stated one stands.
    let order = settled(&reader, b"8=FIX.4.4|35=D|11=A|150=0|10=0|");
    assert_eq!(order.get_by_tag(39), None);
    let stated = settled(&reader, b"8=FIX.4.4|35=8|39=2|150=F|151=60|14=40|10=0|");
    assert_eq!(text(&stated, 39).as_deref(), Some("2"));

    // A status read off the execution type is a status the remainder rule
    // reads: one pass answers both.
    let chained = settled(&reader, b"8=FIX.4.4|35=8|150=0|38=100|14=0|10=0|");
    assert_eq!(text(&chained, 39).as_deref(), Some("0"));
    assert_eq!(chained.by_tag(151).unwrap(), super::decimal("100"));
    assert_eq!(statecode(&chained).as_deref(), Some(state("0").as_str()));
}

#[test]
fn a_value_that_would_not_type_is_filled_in_place() {
    crate::install::installed();
    let reader = reader();
    // `PutOrCall` is a number, so `abc` types as null while the option's own
    // code says it is a call; the answer takes the null's place rather than
    // standing beside it.
    const LINE: &[u8] = b"8=FIX.4.4|35=D|11=A|461=OCXXXX|201=abc|10=0|";
    let held = settled(&reader, LINE);
    // The parse states it: the null the text typed to is where the answer
    // lands, so the column holds one value and not two.
    assert_eq!(integer(&held, 201), Some(1));
    assert_eq!(
        held.as_field()
            .dtype()
            .as_fields()
            .expect("a struct")
            .iter()
            .filter(|child| child.name() == "putorcall")
            .count(),
        1,
        "one column for the tag"
    );
    // The entries are the row read as a tree, so the pair the wire re-emits
    // under that tag is the value the fill landed, not the text that would
    // not type.
    let entry = held
        .entries()
        .iter()
        .find(|entry| entry.tag() == 201)
        .expect("the column the fill landed in");
    assert_eq!(entry.value(), Some("1"));
    assert!(
        String::from_utf8(held.into_bytes(b'|'))
            .unwrap()
            .contains("|201=1|")
    );
}

#[test]
fn the_shipped_native_plan_fills_quotes_pegs_contract_amounts_and_currency_source() {
    crate::install::installed();
    let reader = reader();
    let held = settled(
        &reader,
        b"8=FIX.4.4|35=S|15=CHF|188=1.25|189=0.125|190=1.5|191=-0.25|211=-0.25|231=2|969=0.5|1095=100.5|10=0|",
    );

    assert_eq!(held.by_tag(132).unwrap(), super::decimal("1.375"));
    assert_eq!(held.by_tag(133).unwrap(), super::decimal("1.25"));
    assert_eq!(held.by_tag(839).unwrap(), super::decimal("100.25"));
    assert_eq!(held.by_tag(1146).unwrap(), super::decimal("1"));
    assert_eq!(text(&held, 2897).as_deref(), Some("6"));

    // A digital-asset ticker is a currency ISO 4217 does not list, so no
    // source is stated for it - neither ISO 4217's `6` nor any other.
    let held = settled(&reader, b"8=FIX.4.4|35=S|15=USDT|188=1.25|10=0|");
    assert_eq!(text(&held, 15).as_deref(), Some("USDT"));
    assert_eq!(text(&held, 2897).as_deref(), None);
}

/// The committed dictionary, owned, for the cases that edit a field.
fn committed() -> FixRegistry {
    super::committed_registry().as_ref().clone()
}

#[test]
fn the_native_plan_evaluates_both_sums_and_folds_unicode_text() {
    crate::install::installed();
    let native = reader();
    // `OrderQty(38)` falls back from `CumQty + LeavesQty` to
    // `CumQty + CxlQty`, and both sums are evaluated before the first that
    // is stated answers - here at the widest value the column holds.
    let widest = settled(
        &native,
        b"8=FIX.4.4|35=8|14=99999999999999999999.999999999999999998|151=0.000000000000000001|84=0.000000000000000002|10=0|",
    );
    assert_eq!(
        widest.by_tag(38).unwrap(),
        super::decimal("99999999999999999999.999999999999999999")
    );

    let security = settled(&native, "8=FIX.4.4|35=D|167=Cſ|10=0|".as_bytes());
    assert_eq!(integer(&security, 460), Some(5));
    assert_eq!(text(&security, 461).as_deref(), Some("ESXXXX"));
    let cfi = settled(&native, "8=FIX.4.4|35=D|461=OéFXXX|10=0|".as_bytes());
    assert_eq!(text(&cfi, 167).as_deref(), Some("OOF"));
}

/// The foreign exchange rules: what ISO 10962:2021's non-deliverable
/// attribute and the two legs of a currency pair call for.
#[test]
fn the_native_fx_derivations_answer_the_iso_10962_readings() {
    crate::install::installed();
    let native = reader();
    // The line's body, the tag read and what the rules answer for it.
    let cases: &[(&str, i32, Option<&str>)] = &[
        // Position 6 `N` does not deliver, and neither does a forward
        // priced forward and settled in cash; a contract for difference
        // and a spread bet settle in cash and stay forwards.
        ("461=JFTXFN|", 167, Some("FXNDF")),
        ("461=jftxfn|", 167, Some("FXNDF")),
        ("461=JF\u{e9}XFN|", 167, Some("FXNDF")),
        ("461=JFTXFC|", 167, Some("FXNDF")),
        ("461=JFTXCC|", 167, Some("FXFWD")),
        ("461=JFTXSC|", 167, Some("FXFWD")),
        ("461=JFTXFP|", 167, Some("FXFWD")),
        ("461=SFXXXN|", 167, Some("FXNDS")),
        ("461=SFXXXP|", 167, Some("FXSWAP")),
        ("461=IFXXXP|", 167, Some("FXSPOT")),
        ("461=OCFXXX|", 167, Some("OOF")),
        // The other way, down to the delivery.
        ("167=FXSPOT|", 461, Some("IFXXXP")),
        ("167=FXFWD|", 461, Some("JFTXFP")),
        ("167=FXSWAP|", 461, Some("SFXXXP")),
        ("167=FXNDS|", 461, Some("SFXXXN")),
        ("55=EUR/USD|167=FXNDF|", 461, Some("JFTXFN")),
        // A currency product's two currencies are its two legs.
        ("460=4|15=EUR|", 120, None),
        ("460=4|120=USD|", 15, None),
        ("460=1|15=EUR|", 120, Some("EUR")),
        ("460=1|120=USD|", 15, Some("USD")),
        ("15=EUR|", 120, Some("EUR")),
    ];
    for (body, tag, expected) in cases {
        let line = format!("8=FIX.4.4|35=D|{body}10=0|");
        let held = native.sole_line(line.as_bytes()).expect("a readable line");
        assert_eq!(text(&held, *tag).as_deref(), *expected, "{tag} of {line}");
    }

    // A non-deliverable forward's CFI is a detailed one, which the market
    // keeps, and its product is the currency one.
    let forward = native
        .sole_line(b"8=FIX.4.4|35=D|55=EUR/USD|167=FXNDF|10=0|")
        .expect("a readable line");
    assert_eq!(integer(&forward, 460), Some(4));
    assert_eq!(
        forward.get_cficode().map(|cfi| cfi.as_str()),
        Some("JFTXFN")
    );
    assert!(yggdryl::Cfi::is_detailed("JFTXFN"));
}

#[test]
fn a_registry_states_no_rule_of_its_own() {
    crate::install::installed();
    // A key a registry stores on a field is inert text: `LeavesQty(151)` is
    // what was ordered less what was done, whatever the field says.
    let mut registry = committed();
    let mut leaves = registry.field_by_tag(151).expect("LeavesQty").clone();
    leaves
        .insert_metadata("FIX:derivation", "(orderqty - cumqty) / 10")
        .expect("any text can be stored on a field");
    registry.update(leaves).expect("an inert key is stored");
    let reader = super::fixed_codec(Arc::new(registry));
    let held = settled(&reader, b"8=FIX.4.4|35=8|15=CHF|39=0|38=100|14=20|10=0|");
    assert_eq!(held.by_tag(151).unwrap(), super::decimal("80"));
    assert_eq!(text(&held, 2897).as_deref(), Some("6"));

    // And the committed dictionary carries none: the rules are the crate's.
    let registry = committed();
    let crated = yggdryl_fix::fix_crate_fields().expect("the crate's own fields");
    for field in registry.iter().chain(crated.iter()) {
        for key in ["FIX:derivation", "FIX:replacements"] {
            assert!(!field.has_metadata(key), "{} carries {key}", field.name());
        }
    }
}

#[test]
fn every_registry_fills_by_the_same_rules() {
    crate::install::installed();
    // A registry of five fields holds the targets and the inputs of one
    // rule, and fills by it exactly as the committed dictionary does; one
    // lacking the target's field answers nothing there.
    let committed = committed();
    let held = |tag: i32| {
        let mut field = committed
            .field_by_tag(tag)
            .expect("a standard field")
            .clone();
        FixFieldMut::new(&mut field).remove_codeset();
        field
    };
    let small =
        FixRegistry::from_fields([35, 38, 14, 39, 151].map(held)).expect("a small dictionary");
    let reader = super::fixed_codec(Arc::new(small));
    let line = b"8=FIX.4.4|35=8|39=0|38=100|14=20|32=10|31=2|10=0|";
    let message = settled(&reader, line);
    assert_eq!(message.by_tag(151).unwrap(), super::decimal("80"));
    assert_eq!(
        message.get_by_tag(381),
        None,
        "no GrossTradeAmt field to fill"
    );
}

#[test]
fn an_absent_input_is_silence_and_a_stated_value_is_never_overwritten() {
    crate::install::installed();
    let reader = reader();
    // Every input absent: nothing derives, nothing refuses.
    let bare = settled(&reader, b"8=FIX.4.4|35=8|37=A|10=0|");
    for tag in [
        6, 14, 22, 31, 38, 39, 48, 55, 119, 120, 151, 167, 381, 460, 461, 470,
    ] {
        assert_eq!(bare.get_by_tag(tag), None, "tag {tag}");
    }
    // One input absent of two: the product is silent, not a guess.
    let half = settled(&reader, b"8=FIX.4.4|35=8|37=A|32=10|10=0|");
    assert_eq!(half.get_by_tag(381), None);
    let whole = settled(&reader, b"8=FIX.4.4|35=8|37=A|32=10|31=2.5|10=0|");
    assert_eq!(whole.by_tag(381).unwrap(), super::decimal("25"));

    // A stated value stands whatever the derivation would say, and a stated
    // null is not a stated value: the derivation fills it in place.
    let stated = settled(&reader, b"8=FIX.4.4|35=8|37=A|32=10|31=2.5|381=99|10=0|");
    assert_eq!(stated.by_tag(381).unwrap(), super::decimal("99"));
    let nulled = settled(&reader, b"8=FIX.4.4|35=8|37=A|32=10|31=2.5|381=abc|10=0|");
    assert_eq!(nulled.by_tag(381).unwrap(), super::decimal("25"));
    // The wire re-emits the message as it now stands: the frame, then the
    // fields it lifted in tag order, then its content, then the trailer.
    // What it *derived* - the price and the quantity it is about, read off
    // the fill it stated - is emitted nowhere.
    assert_eq!(
        String::from_utf8(nulled.into_bytes(b'|')).unwrap(),
        "8=FIX.4.4|35=8|31=2.5|32=10|37=A|381=25|59=0|10=0|"
    );
}

#[test]
fn a_chain_resolves_in_one_pass_whatever_order_its_fields_fall_in() {
    crate::install::installed();
    let reader = reader();
    // `securityid` -> `securityidsource` -> `isincode` -> `countryofissue`:
    // the source is read off the identifier, the column off the source, the
    // country off the column.
    let chain = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=GB0002634946|10=0|");
    assert_eq!(text(&chain, 22).as_deref(), Some("4"));
    assert_eq!(isincode(&chain).as_deref(), Some("GB0002634946"));
    assert_eq!(text(&chain, 470).as_deref(), Some("GB"));
    // The other way round, from the alternate a message states instead of
    // a primary: `securityids` -> `securityid` -> `securityidsource`, a
    // lower tag filled off a group.
    let reversed = settled(&reader, &alternate("GB0002634946", "4"));
    assert_eq!(text(&reversed, 48).as_deref(), Some("GB0002634946"));
    assert_eq!(text(&reversed, 22).as_deref(), Some("4"));
    // `cficode` -> `securitytype` -> `product`, and `exectype` ->
    // `ordstatus` -> `leavesqty` -> `state`.
    let typed = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=LRXXXX|10=0|");
    assert_eq!(text(&typed, 167).as_deref(), Some("REPO"));
    assert_eq!(integer(&typed, 460), Some(13));
    let report = settled(&reader, b"8=FIX.4.4|35=8|150=0|38=100|14=0|10=0|");
    assert_eq!(text(&report, 39).as_deref(), Some("0"));
    assert_eq!(report.by_tag(151).unwrap(), super::decimal("100"));
    assert_eq!(statecode(&report).as_deref(), Some(state("0").as_str()));
}

#[test]
fn normalized_market_codes_are_answered_and_columned_while_other_facts_are_not() {
    crate::install::installed();
    // The row carries its stated FIX children - `ExDestination(100)`,
    // `ExecType(150)` as columns, `SecurityID(48)` under its source among its
    // entries - states the identifier they name in `securityids`, and lifts the
    // normalized ISIN and MIC into their dedicated code columns. Other graph
    // answers stay derived rather than becoming columns. The ranked state is
    // an event fact, stated at its own column so a reader sees what the walk
    // folded.
    let reader = reader();
    let schema = yggdryl_fix::fix_schema(reader.registry(), "fix").expect("the fixed schema");
    let line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|22=4|100=XNAS|150=F|10=0|";
    let filled = reader.sole_line(line).expect("a readable line");
    let row = filled.clone().into_row(&schema).expect("a row");
    let column = |name: &str| {
        let at = schema.index_of(name).expect(name);
        row.as_sequence().expect("a row")[at].clone()
    };
    for name in [
        "px",
        "qty",
        "symbolticker",
        "securityid",
        "securityidsource",
    ] {
        assert_eq!(schema.index_of(name), None, "{name} is no column");
    }
    let entry = |key: &str| {
        column(yggdryl_fix::FIXENTRIES_COLUMN)
            .mapping_iter()
            .find(|(held, _)| held.as_str() == Some(key))
            .and_then(|(_, value)| value.as_str().map(str::to_owned))
    };
    assert_eq!(
        column("state"),
        yggdryl::Scalar::State(yggdryl::State::read(&state("F")).expect("a state")),
        "the ranked state is a column of its own"
    );
    assert_eq!(entry("48:securityid").as_deref(), Some("US0378331005"));
    assert_eq!(entry("22:securityidsource").as_deref(), Some("4"));
    assert_eq!(
        yggdryl_market::Identifiers::from_scalar(&column("securityids"))
            .expect("the security identifiers")
            .get_from(&IdKey::base(IdType::Isin)),
        Some("US0378331005")
    );
    assert_eq!(column("exdestination").as_str(), Some("XNAS"));
    assert_eq!(column("exectype").as_str(), Some("F"));
    assert_eq!(column("isincode").as_str(), Some("US0378331005"));
    assert_eq!(column("miccode").as_str(), Some("XNAS"));
    // The normalized columns answer from the stated message children.
    assert_eq!(isincode(&filled).as_deref(), Some("US0378331005"));
    assert_eq!(miccode(&filled).as_deref(), Some("XNAS"));
    assert_eq!(statecode(&filled).as_deref(), Some(state("F").as_str()));
}

#[test]
fn an_unvalidated_primary_under_the_isin_source_falls_through_to_the_alternate() {
    crate::install::installed();
    let reader = reader();
    // A primary no check digit closes, stated under the ISIN source beside
    // an alternate the digit does close: the alternate answers, and the
    // country is read off it.
    let fallen = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|22=4|48=NOTANISIN00|454=1|455=CH0012221716|456=4|10=0|",
    );
    assert_eq!(isincode(&fallen).as_deref(), Some("CH0012221716"));
    assert_eq!(text(&fallen, 470).as_deref(), Some("CH"));
    // A typo in the primary is the same fall-through: one digit off is not
    // that security.
    let typo = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|22=4|48=US0378331006|454=1|455=CH0012221716|456=4|10=0|",
    );
    assert_eq!(isincode(&typo).as_deref(), Some("CH0012221716"));
    // A primary the digit closes answers itself, whatever the alternate says.
    let primary = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|22=4|48=US0378331005|454=1|455=CH0012221716|456=4|10=0|",
    );
    assert_eq!(isincode(&primary).as_deref(), Some("US0378331005"));
    assert_eq!(text(&primary, 470).as_deref(), Some("US"));
    // Neither closes: the alternate of the number's shape is the ISIN as
    // stated, a typo of rank one, and nothing downstream reads a country
    // off a number that does not close.
    let neither = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|22=4|48=NOTANISIN00|454=1|455=CH0012221717|456=4|10=0|",
    );
    assert_eq!(isincode(&neither).as_deref(), Some("CH0012221717"));
    assert_eq!(neither.get_by_tag(470), None);
}

#[test]
fn a_country_of_issue_is_exactly_a_prefix_the_crates_registry_lists() {
    crate::install::installed();
    // ISO 6166 opens a number with two letters, and `CountryOfIssue` answers
    // for exactly the pairs `StringEnum::COUNTRIES` lists: every one of the
    // 676 pairs, closed by its own check digit, answers its prefix where the
    // registry lists it and nothing where it does not.
    let reader = reader();
    let mut listed = 0;
    for first in b'A'..=b'Z' {
        for second in b'A'..=b'Z' {
            let prefix = format!("{}{}", char::from(first), char::from(second));
            let body = format!("{prefix}000000000");
            let digit = Isin::closing_digit(&body).expect("two letters and nine digits close");
            let number = format!("{body}{digit}");
            let line = format!("8=FIX.4.4|35=D|11=A|22=4|48={number}|10=0|");
            let held = reader.sole_line(line.as_bytes()).expect("a readable line");
            assert_eq!(
                isincode(&held).as_deref(),
                Some(number.as_str()),
                "{prefix}"
            );
            let expected = StringEnum::COUNTRIES
                .binary_search(&prefix.as_str())
                .is_ok();
            assert_eq!(
                text(&held, 470).as_deref(),
                expected.then_some(prefix.as_str()),
                "{prefix} answers as the registry lists it"
            );
            listed += usize::from(expected);
        }
    }
    assert_eq!(listed, StringEnum::COUNTRIES.len());
    assert_eq!(listed, 249);
}

#[test]
fn a_report_with_nothing_left_that_states_what_was_canceled_ordered_done_plus_canceled() {
    crate::install::installed();
    // Appendix D's cancel: a canceled order asked for what it did plus what
    // was canceled, and nothing is left. The old hand-laid order derived
    // `LeavesQty` first and answered `CumQty` alone; the rule now says it
    // outright, whether the report states nothing left or states nothing.
    let reader = reader();
    let closed = settled(&reader, b"8=FIX.4.4|35=8|37=A|39=4|14=40|84=60|10=0|");
    assert_eq!(closed.by_tag(38).unwrap(), super::decimal("100"));
    assert_eq!(closed.by_tag(151).unwrap(), super::decimal("0"));
    let stated = settled(&reader, b"8=FIX.4.4|35=8|37=A|39=4|14=40|151=0|84=60|10=0|");
    assert_eq!(stated.by_tag(38).unwrap(), super::decimal("100"));
    let typed = settled(&reader, b"8=FIX.4.4|35=8|37=A|150=4|14=40|84=60|10=0|");
    assert_eq!(typed.by_tag(38).unwrap(), super::decimal("100"));
    assert_eq!(text(&typed, 39).as_deref(), Some("4"));
    // A working report stating what is left orders done plus left, whatever
    // it canceled along the way: a replace that cut the quantity restated
    // what was ordered.
    let working = settled(
        &reader,
        b"8=FIX.4.4|35=8|37=A|39=1|14=40|151=40|84=20|10=0|",
    );
    assert_eq!(working.by_tag(38).unwrap(), super::decimal("80"));
}

#[test]
fn the_fixpoint_reaches_the_chains_one_pass_could_not() {
    crate::install::installed();
    // Each of these fills a value the deleted single pass left absent - and
    // added on its second run, which is what made that pass non-idempotent.
    let reader = reader();
    // A forward price derived from spot and points is a price the worth
    // reads, and the average of one fill.
    let forward = settled(
        &reader,
        b"8=FIX.4.4|35=8|37=A|32=10|194=1.25|195=0.25|10=0|",
    );
    assert_eq!(forward.by_tag(31).unwrap(), super::decimal("1.5"));
    assert_eq!(forward.by_tag(381).unwrap(), super::decimal("15"));
    let average = settled(
        &reader,
        b"8=FIX.4.4|35=8|37=A|32=10|14=10|194=1.25|195=0.25|10=0|",
    );
    assert_eq!(average.by_tag(6).unwrap(), super::decimal("1.5"));
    // What was ordered, read off what was canceled, is what the remainder
    // reads: a new order that canceled nothing yet has everything left.
    let fresh = settled(&reader, b"8=FIX.4.4|35=8|37=A|150=0|14=0|84=100|10=0|");
    assert_eq!(fresh.by_tag(38).unwrap(), super::decimal("100"));
    assert_eq!(text(&fresh, 39).as_deref(), Some("0"));
    assert_eq!(fresh.by_tag(151).unwrap(), super::decimal("100"));
    // A trade over several periods, then the multiplied and the gross
    // quantities read off it.
    let periods = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|32=10|31=3|2353=2|231=5|10=0|",
    );
    assert_eq!(periods.by_tag(2367).unwrap(), super::decimal("20"));
    assert_eq!(periods.by_tag(2370).unwrap(), super::decimal("100"));
    assert_eq!(periods.by_tag(2369).unwrap(), super::decimal("60"));
    assert_eq!(periods.by_tag(2368).unwrap(), super::decimal("50"));
}

#[test]
fn a_typed_read_fires_where_the_old_text_read_could_not() {
    crate::install::installed();
    // `PossDupFlag` is a boolean and `TradingUnitPeriodMultiplier` an
    // integer; the deleted rules read both as text or as a float and never
    // fired. A term reads each as what it is.
    let reader = reader();
    let repeated = settled(
        &reader,
        b"8=FIX.4.4|35=8|37=A|43=Y|52=20240102-10:15:30|10=0|",
    );
    assert_eq!(repeated.by_tag(122).unwrap(), repeated.by_tag(52).unwrap());
    let original = settled(
        &reader,
        b"8=FIX.4.4|35=8|37=A|43=N|52=20240102-10:15:30|10=0|",
    );
    assert_eq!(original.get_by_tag(122), None);
    let periods = settled(&reader, b"8=FIX.4.4|35=D|11=A|32=10|2353=2|10=0|");
    assert_eq!(periods.by_tag(2367).unwrap(), super::decimal("20"));
}

#[test]
fn a_source_code_is_compared_exactly_because_fix_codes_are_case_sensitive() {
    crate::install::installed();
    // `A` is Bloomberg's source; `a` is no code of the set, and the deleted
    // rule's case folding read it as one.
    let reader = reader();
    let bloomberg = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=ABBN SW|22=A|10=0|");
    assert_eq!(text(&bloomberg, 55).as_deref(), Some("ABBN SW"));
    let folded = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=ABBN SW|22=a|10=0|");
    assert_eq!(folded.get_by_tag(55), None);
}

#[test]
fn a_registry_of_a_handful_of_fields_derives_its_market_from_what_it_holds() {
    crate::install::installed();
    // A registry holding none of the standard's fields compiles no rule,
    // so a message enriches without a refusal and without a fill - and the
    // market facts the traits answer are read off the row rather than
    // filled, so they answer whatever the line stated under a tag the
    // registry could still type.
    let registry = FixRegistry::new();
    let reader = super::fixed_codec(Arc::new(registry));
    let held = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|48=US0378331005|22=4|207=XNAS|10=0|")
        .expect("a readable line");
    assert_eq!(isincode(&held).as_deref(), Some("US0378331005"));
    assert_eq!(miccode(&held).as_deref(), Some("XNAS"));
    // No status field states one, so the state is what a new order asks for.
    assert_eq!(
        statecode(&held).as_deref(),
        Some("PENDING_NEW"),
        "a new order asks for one"
    );
}

#[test]
fn a_stream_of_every_shape_costs_nothing_between_messages() {
    crate::install::installed();
    // No shape is recognized and nothing is kept per shape or between
    // messages: the bridge's own capture, every shape it writes, enriches
    // to the same answers message by message in either order, through the
    // stream door twice, over what the stream already enriched - and the
    // stream answers exactly what the one-message door answers, because it
    // carries nothing from one message to the next.
    let reader = super::fixed_codec(super::committed_registry())
        .with_exclude_msgtypes::<[&str; 0], &str>([]);
    let source = Buffer::from_bytes(include_bytes!("../../../tests/support/ulbridge.log").to_vec())
        .with_media_type(
            Url::from_str("file:///ulbridge.log")
                .expect("a URL")
                .media_type(),
        );
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl_fix::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    let lines: Vec<TextLine> = read_text_lines(&source, &options)
        .expect("a line reader")
        .map(|line| line.expect("a line"))
        .collect();
    let messages: Vec<FixMsg> = lines
        .iter()
        .filter_map(|line| reader.parse_text_line(line).ok())
        .flatten()
        .filter_map(Result::ok)
        .collect();
    // 94: every JSON document the capture holds - the seven Jolokia
    // answers, the two wildcards and the error among them, and the
    // statistics line - is one `unknown` row, never one per plugin it named
    // and never none - and 57 more: the execution the parse splits off each
    // execution report of a fill (A12), and the one of side `UKNW` the
    // trade capture, stating no `Side(54)`, splits off.
    assert_eq!(messages.len(), 94 + 57, "the corpus");
    let forward: Vec<FixMsg> = messages.to_vec();
    let mut backward: Vec<FixMsg> = messages.iter().rev().cloned().collect();
    backward.reverse();
    assert_eq!(forward, backward, "order changes nothing a message derives");
    // A parse fills as it reads, so the stream is the messages themselves:
    // reading the same lines twice answers the same messages.
    let streamed = messages.clone();
    let again = messages;
    assert_eq!(streamed, again, "a second stream answers the first");
    assert_eq!(
        forward, streamed,
        "the stream carries nothing between messages: a row naming a plugin \
         some document configured takes nothing from that document"
    );
    let settled = streamed.clone();
    assert_eq!(
        streamed, settled,
        "a pass over what was filled changes nothing"
    );
}

/// Twelve messages over four hours in instant order: three orders placed,
/// replaced and filled, an hour of each order's life apart.
fn four_hours() -> Vec<FixMsg> {
    let codec = super::fixed_codec(super::committed_registry());
    let mut lines = Vec::new();
    for hour in 10..14 {
        for (at, order) in [(5, "A"), (20, "B"), (40, "C")] {
            let status = if hour == 13 { "2" } else { "0" };
            lines.push(format!(
                "8=FIX.4.4|35=8|52=20260102-{hour}:{at:02}:00|37={order}|11={order}-{hour}|17=E-{order}-{hour}|150=0|39={status}|54=1|55=AAPL|38=10|44=5|10=0|"
            ));
        }
    }
    lines
        .iter()
        .map(|line| codec.parse_fix_line(line.as_bytes()).expect("a message"))
        .collect()
}

/// What a walk answered, as its chain reads: the identity, the place and
/// the predecessor of each message.
fn walked(
    codec: &FixCodec,
    messages: Vec<FixMsg>,
) -> Vec<(yggdryl::Uuid, u64, Option<yggdryl::Uuid>)> {
    use yggdryl::graph::Element;
    codec
        .lifecycle(messages)
        .map(|message| {
            let message = message.expect("a walked message");
            (
                message.get_uuid(),
                message.get_seqnum(),
                message.get_prevuuid(),
            )
        })
        .collect()
}

#[test]
fn a_sorted_lifecycle_walks_an_ordered_source_as_the_whole_sort_does() {
    crate::install::installed();
    let codec = super::fixed_codec(super::committed_registry());
    let whole = walked(&codec, four_hours());
    let sorted = walked(&codec.clone().with_sorted_lifecycle(true), four_hours());
    assert!(codec.clone().with_sorted_lifecycle(true).sorted_lifecycle());
    assert!(!codec.sorted_lifecycle());
    assert_eq!(whole.len(), 12);
    assert_eq!(sorted, whole);
}

#[test]
fn a_sorted_lifecycle_walks_an_hour_before_it_reads_the_last() {
    crate::install::installed();
    use std::cell::Cell;
    use std::rc::Rc;
    let messages = four_hours();
    let pulls = Rc::new(Cell::new(0_usize));
    let counted = Rc::clone(&pulls);
    let source = messages.into_iter().map(move |message| {
        counted.set(counted.get() + 1);
        Ok::<_, yggdryl::Error>(message)
    });
    let codec = super::fixed_codec(super::committed_registry()).with_sorted_lifecycle(true);
    let mut walk = codec.lifecycle(source);
    walk.next().expect("the first message").expect("walked");
    // The first hour is walked once the stream has read two hours past it:
    // its three messages, the next hour's three and the first of the hour
    // after - never the whole capture.
    assert_eq!(pulls.get(), 7);
    assert_eq!(walk.count(), 11);
    assert_eq!(pulls.get(), 12);
}

#[test]
fn the_lifecycle_yields_an_identity_once_within_its_dedup_window() {
    crate::install::installed();
    let codec = super::fixed_codec(super::committed_registry());
    assert_eq!(FixCodec::DEFAULT_DEDUP_WINDOW_MS, 60_000);
    assert_eq!(codec.dedup_window_ms(), Some(60_000));
    for (window, read) in [(1, Some(1)), (0, None), (-1, None)] {
        assert_eq!(
            codec.clone().with_dedup_window_ms(window).dedup_window_ms(),
            read
        );
    }
    let parse = |line: String| codec.parse_fix_line(line.as_bytes()).expect("a message");
    let order = |at: &str, clordid: &str, sequence: u64| {
        parse(format!(
            "8=FIX.4.4|35=D|49=S|56=T|34={sequence}|52=20260102-{at}|11={clordid}|55=AAPL|54=1|38=100|10=0|"
        ))
    };
    // The order delivered again under another sequence is another delivery
    // of the same event: the walk restates the live order with it, so it
    // answers the order's own identity - after another order, not beside
    // its twin.
    let messages = vec![
        order("10:15:30", "A1", 1),
        order("10:15:30", "B1", 2),
        order("10:15:30", "A1", 3),
    ];
    let every = walked(&codec.clone().with_dedup_window_ms(0), messages.clone());
    assert_eq!(every.len(), 3);
    assert_eq!(every[2], every[0], "the repeat restates the order");
    assert_eq!(walked(&codec, messages), every[..2]);

    // The window is event time: a repeat read after the walk has moved
    // further than the window past it is yielded again. A sorted source
    // walks a message dated before an hour it already walked where it
    // arrives, after what the walk yielded since.
    let late = vec![
        order("10:15:30", "A1", 1),
        order("12:00:00", "X1", 2),
        order("14:00:00", "Z1", 3),
        order("10:15:30", "A1", 4),
    ];
    let sorted = codec.clone().with_sorted_lifecycle(true);
    let minute = walked(&sorted, late.clone());
    assert_eq!(minute.len(), 4, "an hour and forty-five minutes on");
    assert_eq!(minute[2], minute[0]);
    let hours = walked(&sorted.with_dedup_window_ms(2 * 3_600_000), late);
    assert_eq!(hours.len(), 3);
    assert_eq!(hours, [minute[0], minute[1], minute[3]]);
}

/// A twin of a message that took its chain's side restates to the live
/// one's identity: what the walk wrote into the live one's content it
/// writes into the twin's, so the two digest alike and the window yields
/// that identity once.
#[test]
fn a_twin_of_an_acknowledgement_that_took_its_chains_side_is_one_identity() {
    crate::install::installed();
    use yggdryl::graph::Element;

    let codec = super::fixed_codec(super::committed_registry());
    let lines: [&[u8]; 3] = [
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30.250|11=A1|55=AAPL|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=1|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|55=AAPL|10=0|",
        // The acknowledgement delivered again under another sequence.
        b"8=FIX.4.4|35=8|49=T|56=S|34=2|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|55=AAPL|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("three messages");
    let every: Vec<FixMsg> = codec
        .clone()
        .with_dedup_window_ms(0)
        .lifecycle(parsed.clone())
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    assert_eq!(every.len(), 3, "both deliveries are walked");
    assert_eq!(text(&every[1], 54).as_deref(), Some("BUYS"));
    assert_eq!(text(&every[2], 54).as_deref(), Some("BUYS"));
    assert_eq!(
        every[2].get_uuid(),
        every[1].get_uuid(),
        "a twin finalizes to the live one's identity"
    );
    let once = walked(&codec, parsed);
    assert_eq!(once.len(), 2, "the window yields that identity once");
    assert_eq!(once[1].0, every[1].get_uuid());
}

/// An acknowledgement delivered again after the fill that moved its chain
/// on at the same instant is another statement of the acknowledgement, not
/// a step after the fill: it restates to the acknowledgement's identity,
/// which the window yields once.
#[test]
fn an_acknowledgement_delivered_again_after_its_fill_adds_no_step() {
    crate::install::installed();
    use yggdryl::graph::Element;
    use yggdryl_market::MarketDataKind;

    let codec = super::fixed_codec(super::committed_registry());
    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30.000|11=A1|55=AAPL|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=1|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=2|52=20260102-10:15:30.500|11=A1|37=O1|17=E1|150=F|39=1|14=10|151=90|31=10|32=10|54=1|55=AAPL|10=0|",
        // The acknowledgement delivered again, after the fill of its instant.
        b"8=FIX.4.4|35=8|49=T|56=S|34=3|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|54=1|55=AAPL|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("the messages");
    let orders = |codec: &FixCodec| {
        codec
            .lifecycle(parsed.clone())
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("the walk")
            .into_iter()
            .filter(|held| held.marketdatakind() == MarketDataKind::Order)
            .collect::<Vec<_>>()
    };
    let every = orders(&codec.clone().with_dedup_window_ms(0));
    let [order, ack, fill, again] = every.as_slice() else {
        panic!("four order messages, not {}", every.len())
    };
    assert_eq!(ack.get_prevuuid(), Some(order.get_uuid()));
    assert_eq!(fill.get_prevuuid(), Some(ack.get_uuid()));
    assert_eq!(again.get_uuid(), ack.get_uuid());
    assert_eq!(*again.get_state(), State::New);
    let once = orders(&codec);
    assert_eq!(
        once.len(),
        3,
        "the order, its acknowledgement and its fill's report: {:?}",
        once.iter()
            .map(|held| *held.get_state())
            .collect::<Vec<_>>()
    );
}

/// An acknowledgement delivered again after the fill that ended its chain
/// at the same instant is another statement of the acknowledgement, as it
/// is after a fill that moved the chain on, and the fill delivered again is
/// another statement of the fill: the order's chain is not started afresh,
/// and the window yields each once.
#[test]
fn an_acknowledgement_delivered_again_after_the_fill_that_ended_its_chain_adds_no_step() {
    crate::install::installed();
    use yggdryl::graph::Element;
    use yggdryl_market::MarketDataKind;

    let codec = super::fixed_codec(super::committed_registry());
    let lines: [&[u8]; 5] = [
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30.000|11=A1|55=AAPL|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=1|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=2|52=20260102-10:15:30.500|11=A1|37=O1|17=E1|150=F|39=2|14=100|151=0|31=10|32=100|54=1|55=AAPL|10=0|",
        // The acknowledgement delivered again, after the fill that ended
        // its chain at its instant, then that fill.
        b"8=FIX.4.4|35=8|49=T|56=S|34=3|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=4|52=20260102-10:15:30.500|11=A1|37=O1|17=E1|150=F|39=2|14=100|151=0|31=10|32=100|54=1|55=AAPL|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("the messages");
    let orders = |codec: &FixCodec| {
        codec
            .lifecycle(parsed.clone())
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("the walk")
            .into_iter()
            .filter(|held| held.marketdatakind() == MarketDataKind::Order)
            .collect::<Vec<_>>()
    };
    let every = orders(&codec.clone().with_dedup_window_ms(0));
    let [order, ack, fill, again, filled_again] = every.as_slice() else {
        panic!("five order messages, not {}", every.len())
    };
    assert_eq!(ack.get_prevuuid(), Some(order.get_uuid()));
    assert_eq!(fill.get_prevuuid(), Some(ack.get_uuid()));
    assert_eq!(*fill.get_state(), State::Filled);
    assert_eq!(again.get_uuid(), ack.get_uuid());
    assert_eq!(again.get_prevuuid(), Some(order.get_uuid()));
    assert_eq!(filled_again.get_uuid(), fill.get_uuid());
    assert_eq!(filled_again.get_prevuuid(), Some(ack.get_uuid()));
    let once = orders(&codec);
    assert_eq!(
        once.iter().map(Element::get_uuid).collect::<Vec<_>>(),
        [order, ack, fill].map(Element::get_uuid),
        "the order, its acknowledgement and its fill's report"
    );
}

#[test]
fn a_grid_view_is_never_a_repeat_of_what_it_views() {
    crate::install::installed();
    // A view at the live message's own instant derives the live message's
    // identity; it is a view, one per tick and chain, and the window
    // neither drops nor remembers it.
    use yggdryl::graph::Element;
    let codec = super::fixed_codec(super::committed_registry()).with_snapshot_ns(1_000_000_000);
    let message = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|52=20260102-10:15:30|10=0|")
        .expect("a message");
    let walked: Vec<FixMsg> = codec
        .lifecycle([message])
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    assert_eq!(walked.len(), 2, "the order and its view at its own tick");
    assert_eq!(walked[1].get_snapunix(), Some(walked[0].get_transunix()));
    assert_eq!(walked[1].get_uuid(), walked[0].get_uuid());
    // The view is stamped again over the live code rather than settled: it
    // is what settling a copy of it answers, under its chain's cross
    // element.
    let mut settled = walked[1].clone();
    settled.finalize();
    settled.set_crossuuid(walked[1].get_crossuuid());
    assert_eq!(walked[1], settled);
    assert_eq!(walked[1].anomalies(), settled.anomalies());
}

/// A cancel reject stating no `Side(54)` joins the one live side of its
/// order and states that side from then on: in the message the walk yields
/// and in the row it lands as, so a book folding the row reads it.
#[test]
fn a_side_less_follower_states_the_side_of_the_chain_it_joins() {
    crate::install::installed();
    use yggdryl::graph::Element;
    use yggdryl_market::Side;
    let registry = super::committed_registry();
    let codec = super::fixed_codec(std::sync::Arc::clone(&registry));
    let lines = [
        "8=FIX.4.2|35=D|49=B|56=S|34=70|52=20260814-21:50:00|11=C-1|55=2454|54=2|38=100|40=2|44=10|60=20260814-21:50:00|10=0|",
        "8=FIX.4.2|35=8|49=S|56=B|34=71|52=20260814-21:50:01|11=C-1|37=O-1|17=X1|150=0|39=0|54=2|55=2454|38=100|44=10|151=100|14=0|6=0|60=20260814-21:50:01|10=0|",
        "8=FIX.4.2|35=9|49=S|56=B|34=72|52=20260814-21:59:47|11=C-2|37=O-1|41=C-1|39=8|434=1|60=20260814-21:59:46|10=0|",
    ];
    let messages: Vec<FixMsg> = lines
        .iter()
        .map(|line| codec.parse_fix_line(line.as_bytes()).expect("a message"))
        .collect();
    assert_eq!(messages[2].get_side(), Side::Unknown);
    let walked: Vec<FixMsg> = codec
        .lifecycle(messages)
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    let reject = walked.last().expect("the reject");
    assert!(reject.get_prevuuid().is_some(), "it follows its order");
    assert_eq!(reject.get_side(), Side::Sell);
    assert_eq!(reject.get_crosscode(), walked[0].get_crosscode());
    assert_eq!(text(reject, 54).as_deref(), Some("SELL"));
    let wire = String::from_utf8(reject.into_bytes(b'|')).expect("a text wire");
    assert!(wire.contains("|54=2|"), "{wire}");
    let schema = yggdryl_fix::fix_schema(&registry, "fix").expect("the fixed schema");
    let again = FixMsg::from_row(
        std::sync::Arc::clone(&registry),
        &schema,
        &reject.into_row(&schema).expect("a row"),
    )
    .expect("the row reads");
    assert_eq!(again.get_side(), Side::Sell);
}

/// D41, decision 25: a current message matches the previous alive message
/// of its kind and side sharing one identifier of the same type and value,
/// whatever source spelled it. An order a bridge placed under its own
/// `OMS_OrderID` - read as `oms:orderid`, filling the base `orderid` - is
/// joined by the venue's report stating only `OrderID(37)` of that value
/// and no `ClOrdID(11)`: two cross codes, one chain, re-keyed onto the
/// order's, the order's `ClOrdID` and `oms:orderid` propagated onto the
/// report; the same report on the other side joins nothing.
#[test]
fn a_venue_report_stating_only_the_bridges_order_identifier_joins_the_order() {
    crate::install::installed();
    use yggdryl::graph::Element;
    use yggdryl_market::Side;
    let codec = reader();
    let placed = b"8=FIX.4.4|35=D|49=B|56=S|34=10|52=20260921-10:00:00|11=C-1|55=AAPL|54=1|38=100|40=2|44=10|OMS_ORDERID=X-1|60=20260921-10:00:00|10=0|";
    let report = |side: &str| {
        format!(
            "8=FIX.4.4|35=8|49=S|56=B|34=11|52=20260921-10:00:01|37=X-1|17=E1|150=0|39=0|55=AAPL|54={side}|38=100|44=10|151=100|14=0|6=0|60=20260921-10:00:01|10=0|"
        )
    };
    let parsed = [placed.to_vec(), report("1").into_bytes()]
        .map(|line| codec.sole_line(&line).expect("a message"));
    assert_eq!(
        parsed[0]
            .get_identifiers()
            .get_from(&"oms:orderid".parse::<IdKey>().unwrap()),
        Some("X-1")
    );
    assert_eq!(
        parsed[0].get_identifiers().get(&IdType::OrderId),
        Some("X-1")
    );
    assert_eq!(parsed[1].get_identifiers().get(&IdType::ClOrdId), None);
    assert_eq!(parsed[0].get_crosscode(), "10:1:C-1");
    assert_eq!(parsed[1].get_crosscode(), "10:1:X-1");
    let walked = codec
        .lifecycle(parsed.clone())
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the walk");
    let [order, ack] = walked.as_slice() else {
        panic!("the order and its report, not {}", walked.len())
    };
    assert_eq!(ack.get_prevuuid(), Some(order.get_uuid()), "joined");
    assert_eq!(ack.get_crosscode(), "10:1:C-1", "re-keyed onto the order's");
    assert_eq!(ack.get_crossuuid(), order.get_crossuuid());
    assert_eq!(ack.get_identifiers().get(&IdType::ClOrdId), Some("C-1"));
    assert_eq!(
        ack.get_identifiers()
            .get_from(&"oms:orderid".parse::<IdKey>().unwrap()),
        Some("X-1"),
        "the bridge's key propagates"
    );
    assert_eq!(
        text(ack, 11),
        Some("C-1".to_owned()),
        "the predecessor's ClOrdID(11) is written onto a follower stating none"
    );

    // The other side shares the identifier and is another chain.
    let sell = codec.sole_line(report("2").as_bytes()).expect("a message");
    assert_eq!(sell.get_side(), Side::Sell);
    let walked = codec
        .lifecycle([parsed[0].clone(), sell])
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the walk");
    assert_eq!(walked[1].get_prevuuid(), None);
    assert_eq!(walked[1].get_crosscode(), "10:2:X-1");
}

/// A quote is unsided, so the side it tags is its own: a status report
/// following it stating no `Side(54)` takes none from the quote - neither in
/// the message nor on its wire - and takes the quote's legs instead.
#[test]
fn a_quote_follower_inherits_no_side() {
    crate::install::installed();
    use yggdryl::graph::Element;
    use yggdryl_market::Side;

    let codec = super::fixed_codec(super::committed_registry());
    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=S|49=S|56=T|34=1|52=20260102-10:15:30|117=Q1|55=AAPL|54=1|132=99|134=10|10=0|",
        b"8=FIX.4.4|35=AI|49=T|56=S|34=2|52=20260102-10:15:31|117=Q1|55=AAPL|297=0|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("two messages");
    assert_eq!(parsed.len(), 2, "a one-sided quote is one message");
    let chained: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    let status = &chained[1];
    assert_eq!(status.get_prevuuid(), Some(chained[0].get_uuid()));
    assert_eq!(status.get_crosscode(), chained[0].get_crosscode());
    assert_eq!(status.get_side(), Side::Unknown);
    assert_eq!(text(status, 54), None);
    assert_eq!(
        (status.get_bidpx(), status.get_bidqty()),
        (Some("99".parse().unwrap()), Some("10".parse().unwrap())),
        "the leg it states nothing of is its quote's"
    );
}

/// The acknowledgement the lifecycle page walks names no side: the side it
/// takes from its order is written to its wire, beside no stamp.
#[test]
fn the_side_an_acknowledgement_takes_from_its_order_is_written_to_its_wire() {
    crate::install::installed();
    use yggdryl::graph::Element;
    use yggdryl_market::Side;

    let codec = super::fixed_codec(super::committed_registry());
    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|52=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-10:15:30.500|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("two messages");
    let chained: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    let ack = &chained[1];
    assert_eq!(ack.get_prevuuid(), Some(chained[0].get_uuid()));
    assert_eq!(ack.get_side(), Side::Buy);
    let wire = ack.into_text('|').expect("a text wire");
    assert!(wire.contains("|54=1|"), "{wire}");
    assert!(!wire.contains("65014="), "no stamp is a field");
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::graph::{Element, Event};
    use yggdryl::holder::Buffer;
    use yggdryl::text::{TextOptions, read_text_lines};
    use yggdryl::{Timezone, Url};
    use yggdryl_fix::FixMsg;
    use yggdryl_fix::internals::enrich::dated_by_transaction_whole;
    use yggdryl_market::graph::Market;

    /// A redated message settles what its clock moves alone, and over a
    /// parsed message that is the whole pass's answer: every message of
    /// the bridge's capture, read as bare frames and as the capture's own
    /// lines, redated both ways, answers one message - most of them moved.
    #[test]
    fn a_redated_message_settles_its_clock_as_the_whole_pass_does() {
        crate::install::installed();
        let codec = super::super::fixed_codec(super::super::committed_registry())
            .with_exclude_msgtypes::<[&str; 0], &str>([]);
        let log = include_bytes!("../../../tests/support/ulbridge.log");
        let bodies: Vec<Vec<u8>> = log
            .split(|byte| *byte == b'\n')
            .filter_map(|line| {
                let at = line.windows(2).position(|pair| pair == b") ")? + 2;
                Some(line[at..].to_vec())
            })
            .filter(|body| !body.is_empty())
            .collect();
        let framed: Vec<FixMsg> = codec.parse_lines(&bodies).filter_map(Result::ok).collect();
        let source = Buffer::from_bytes(log.to_vec()).with_media_type(
            Url::from_str("file:///ulbridge.log")
                .expect("a URL")
                .media_type(),
        );
        let mut options = TextOptions::new()
            .try_with_rowheader(yggdryl_fix::ULBRIDGE_ROWHEADER)
            .expect("the bridge's row header compiles")
            .with_timezone(Timezone::UTC);
        options.start_rownum = Some(1);
        options.parse_mimetype = true;
        let lined: Vec<FixMsg> = codec
            .clone()
            .with_capture_names(options.capture_names())
            .parse_text_lines(read_text_lines(&source, &options).expect("the capture's lines"))
            .filter_map(Result::ok)
            .collect();
        for messages in [framed, lined] {
            assert_eq!(messages.len(), 94 + 57);
            let mut moved = 0;
            for message in messages {
                let settled = message.clone().dated_by_transaction();
                let whole = dated_by_transaction_whole(message.clone());
                assert_eq!(settled, whole, "{}", message.header().msgtype());
                assert_eq!(settled.get_uuid(), whole.get_uuid());
                assert_eq!(settled.get_execunix(), whole.get_execunix());
                moved += usize::from(settled.get_transunix() != message.get_transunix());
            }
            assert!(moved > 100, "{moved} moved");
        }
    }
}

#[test]
fn a_repeat_the_walk_drops_takes_no_place_and_a_walked_stream_answers_itself() {
    crate::install::installed();
    use yggdryl::graph::Element;
    let codec = super::fixed_codec(super::committed_registry());
    let parse = |line: &str| codec.parse_fix_line(line.as_bytes()).expect("a message");
    // The order, its retransmission a second later - one delivery, dated by
    // its resend - and another order at that second: the dropped resend
    // takes no place, so the other order is the first at its instant.
    let messages = vec![
        parse("8=FIX.4.4|35=D|49=S|56=T|34=7|52=20260102-10:15:30|11=A|55=AAPL|54=1|38=100|10=0|"),
        parse(
            "8=FIX.4.4|35=D|49=S|56=T|34=7|43=Y|52=20260102-10:15:31|122=20260102-10:15:30|11=A|55=AAPL|54=1|38=100|10=0|",
        ),
        parse("8=FIX.4.4|35=D|49=S|56=T|34=9|52=20260102-10:15:31|11=B|55=MSFT|54=1|38=100|10=0|"),
    ];
    let chained: Vec<FixMsg> = codec
        .lifecycle(messages)
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    assert_eq!(chained.len(), 2);
    assert_eq!(chained[1].get_seqnum(), 0);
    let again: Vec<FixMsg> = codec
        .lifecycle(chained.clone())
        .collect::<yggdryl::Result<_>>()
        .expect("the walk again");
    assert_eq!(again, chained);

    // Orders sharing one deadline, and an order at that very instant: the
    // walk hands the expirations over first there, each at the next place,
    // and the order after them; they stand where it placed them, so the
    // walk over its own answer answers it again.
    let dated = |sequence: u64, at: &str, clordid: &str| {
        parse(&format!(
            "8=FIX.4.4|35=D|49=S|56=T|34={sequence}|52=20260102-{at}|126=20260102-10:15:32|11={clordid}|55=AAPL|54=1|38=100|10=0|"
        ))
    };
    let messages = vec![
        dated(1, "10:15:30", "E1"),
        dated(2, "10:15:31", "E2"),
        parse("8=FIX.4.4|35=D|49=S|56=T|34=3|52=20260102-10:15:32|11=S1|55=IBM|54=1|38=100|10=0|"),
    ];
    let chained: Vec<FixMsg> = codec
        .lifecycle(messages)
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    let deadline = chained
        .iter()
        .filter(|message| message.get_transunix() == chained[2].get_transunix())
        .map(|message| (*message.get_state(), message.get_seqnum()))
        .collect::<Vec<_>>();
    assert_eq!(
        deadline,
        [
            (State::Expired, 0),
            (State::Expired, 1),
            (State::PendingNew, 2)
        ],
    );
    let again: Vec<FixMsg> = codec
        .lifecycle(chained.clone())
        .collect::<yggdryl::Result<_>>()
        .expect("the walk again");
    assert_eq!(
        again
            .iter()
            .map(|held| (held.get_uuid(), held.get_seqnum()))
            .collect::<Vec<_>>(),
        chained
            .iter()
            .map(|held| (held.get_uuid(), held.get_seqnum()))
            .collect::<Vec<_>>(),
    );
}

/// A message that follows another in the lifecycle takes every key of its
/// chain's metadata it does not state, its own values standing - the
/// bridge's namespaced keys a message's metadata holds - while its
/// identifiers stay those its fields state and its dictionary follows.
#[test]
fn a_following_message_takes_the_metadata_keys_of_its_chain_it_does_not_state() {
    crate::install::installed();
    let codec = reader();
    let parsed = [
        &b"8=FIX.4.4|35=8|52=20260921-10:00:00|11=C1|37=O-1|17=E-1|150=0|39=0|55=AAPL|54=1|38=5|DESK.NAME=EQ|DESK.BOOK=B1|10=0|"[..],
        &b"8=FIX.4.4|35=8|52=20260921-10:00:01|11=C1|37=O-1|17=E-2|150=6|39=6|55=AAPL|54=1|38=5|DESK.NAME=FX|10=0|"[..],
    ]
    .map(|line| codec.sole_line(line).expect("a report"));
    assert!(!parsed[1].get_metadata().contains_key("desk.book"));
    let walked = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("a readable lifecycle");
    let [first, second] = walked.as_slice() else {
        panic!("two statements, not {}", walked.len())
    };
    assert_eq!(
        second.get_prevuuid(),
        Some(yggdryl::graph::Element::get_uuid(first))
    );
    let metadata: Vec<_> = second
        .get_metadata()
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    assert_eq!(metadata, [("desk.book", "B1"), ("desk.name", "FX")]);
    // The execution's identifier is its own, never the one it follows.
    assert_eq!(second.get_identifiers().get(&IdType::ExecId), Some("E-2"));
}

/// A follower naming no security carries its chain's security identifiers
/// whole - read off `SecurityID(48)` under `SecurityIDSource(22)` and the
/// `SecAltIDGrp(454)`, the CUSIP the ISIN embeds included - writing no
/// field; one naming another ISIN is another instrument, and since
/// decision 25 matches a message with a chain of its instrument where both
/// state one, it is another chain whatever identifiers it shares, and
/// takes none of them.
#[test]
fn a_follower_stating_no_security_takes_its_chains_identifiers_and_another_isin_takes_none() {
    crate::install::installed();
    use yggdryl::graph::Element;

    let codec = reader();
    let lines: [&[u8]; 3] = [
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|48=US0378331005|22=4|454=1|455=BBG000B9XRY4|456=S|54=1|38=10|44=100|10=0|",
        b"8=FIX.4.4|35=8|52=20260921-10:00:01|11=C1|37=O1|150=0|39=0|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|52=20260921-10:00:02|11=C1|37=O1|150=D|39=0|54=1|55=MSFT|48=US5949181045|22=4|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("three messages");
    let chained: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    let [order, ack, other] = chained.as_slice() else {
        panic!("three messages, not {}", chained.len())
    };
    let ids = order.get_securityids();
    assert_eq!(
        ids.get_from(&IdKey::base(IdType::Isin)),
        Some("US0378331005")
    );
    assert_eq!(
        ids.get_from(&IdKey::base(IdType::Figi)),
        Some("BBG000B9XRY4")
    );
    assert_eq!(ids.get(&IdType::Cusip), Some("037833100"));
    assert_eq!(ack.get_prevuuid(), Some(order.get_uuid()));
    assert_eq!(ack.get_securityids(), ids, "the chain's, whole");
    for tag in [48, 22, 454] {
        assert!(
            ack.get_by_tag(tag).is_none_or(|held| held.is_null()),
            "tag {tag}: carried, never written"
        );
    }
    assert_eq!(
        other.get_prevuuid(),
        None,
        "another instrument under the order's identifiers: another chain"
    );
    assert_eq!(
        other.get_crosscode(),
        "10:1:O1",
        "its own code, re-keyed onto nothing"
    );
    assert_eq!(other.get_isincode(), Some("US5949181045"));
    assert_eq!(
        other.get_securityids().get(&IdType::Cusip),
        Some("594918104")
    );
    assert_eq!(
        other.get_securityids().get(&IdType::Figi),
        None,
        "another instrument takes none"
    );
}

/// A follower naming no party carries its chain's party ids - the
/// `Parties(453)` group's and the `Account(1)` - as its own word, writing
/// neither field.
#[test]
fn a_follower_naming_no_party_takes_its_chains_parties_and_writes_none() {
    crate::install::installed();
    use yggdryl::graph::Element;

    let codec = reader();
    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|1=ACC-9|453=1|448=T-1|447=D|452=12|55=AAPL|54=1|38=10|44=100|10=0|",
        b"8=FIX.4.4|35=8|52=20260921-10:00:01|11=C1|37=O1|150=0|39=0|54=1|55=AAPL|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("two messages");
    let chained: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    let [order, ack] = chained.as_slice() else {
        panic!("two messages, not {}", chained.len())
    };
    assert_eq!(
        order.get_partyids().get(&IdType::ExecutingTrader),
        Some("T-1")
    );
    assert_eq!(order.get_partyids().get(&IdType::Account), Some("ACC-9"));
    assert_eq!(ack.get_prevuuid(), Some(order.get_uuid()));
    assert_eq!(ack.get_crosscode(), "10:1:C1");
    assert_eq!(
        ack.get_partyids(),
        order.get_partyids(),
        "the chain's parties"
    );
    for tag in [1, 453] {
        assert!(
            ack.get_by_tag(tag).is_none_or(|held| held.is_null()),
            "tag {tag}: held as the message's word, written to no field"
        );
    }
}

/// The parentage a walk gives the identifiers of a chain: each base a
/// message states takes its parents - the value it held before it last
/// changed, back to the chain's first - from the message before it.
mod parentage {
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event};
    use yggdryl_fix::{FixCodec, FixFieldMut, FixMsg};
    use yggdryl_market::IdType;
    use yggdryl_market::graph::{Market, Operation};

    use super::SoleMessage;

    fn word(text: &str) -> IdType {
        text.parse().expect("a type")
    }

    /// The chain `lines` state, each walked once, in the order given.
    fn walked(codec: &FixCodec, lines: &[&[u8]]) -> Vec<FixMsg> {
        let messages: Vec<FixMsg> = lines
            .iter()
            .map(|line| codec.sole_line(line).expect("a readable line"))
            .collect();
        codec
            .lifecycle(messages)
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("a readable lifecycle")
    }

    /// What the message's identifiers hold under the `fix` source.
    fn fix(message: &FixMsg, kind: &str) -> Option<String> {
        message
            .get_identifiers()
            .get_from(&yggdryl_market::IdKey::base(word(kind)))
            .map(ToOwned::to_owned)
    }

    /// An order replaced twice: each replacement names the client order
    /// identifier it replaced, and it is that - FIX's `OrigClOrdID(41)` - the
    /// chain's parent of `clordid`. The two replacements join the order's
    /// chain through it.
    #[test]
    fn an_order_replaced_twice_carries_the_previous_client_order_identifier() {
        crate::install::installed();
        let codec = super::reader();
        let chain = walked(
            &codec,
            &[
                b"8=FIX.4.4|35=D|52=20260102-10:15:30|11=A|55=AAPL|54=1|38=1|40=2|10=0|",
                b"8=FIX.4.4|35=G|52=20260102-10:15:31|11=B|41=A|55=AAPL|54=1|38=1|40=2|10=0|",
                b"8=FIX.4.4|35=G|52=20260102-10:15:32|11=C|41=B|55=AAPL|54=1|38=1|40=2|10=0|",
            ],
        );
        let [first, second, third] = chain.as_slice() else {
            panic!("three statements, not {}", chain.len())
        };
        assert_eq!(fix(first, "clordid").as_deref(), Some("A"));
        assert_eq!(fix(first, "origclordid"), None, "an order's first has none");
        assert_eq!(fix(second, "clordid").as_deref(), Some("B"));
        assert_eq!(fix(second, "origclordid").as_deref(), Some("A"));
        assert_eq!(fix(third, "clordid").as_deref(), Some("C"));
        assert_eq!(fix(third, "origclordid").as_deref(), Some("B"));
        // One chain: the replacements are joined to the first by the value
        // their parent names, and take its cross code.
        assert_eq!(first.get_crosscode(), "10:1:A");
        assert_eq!(second.get_crosscode(), first.get_crosscode());
        assert_eq!(third.get_crosscode(), first.get_crosscode());
        assert_eq!(second.get_crossuuid(), first.get_crossuuid());
        assert_eq!(third.get_crossuuid(), first.get_crossuuid());
    }

    /// An order whose `OrderID(37)` changes along its chain, A then B, C and
    /// D: the last states `parentorderid` C, the value it held before it last
    /// changed, and `origorderid` A, the value its chain first stated - and
    /// each message before it the same, step by step.
    #[test]
    fn an_order_identifier_that_changes_carries_its_previous_and_its_first_value() {
        crate::install::installed();
        let codec = super::reader();
        let report = |at: u32, orderid: &str, execid: &str| {
            format!(
                "8=FIX.4.4|35=8|52=20260102-10:15:{:02}|11=CL1|37={orderid}|17={execid}|150=0|39=0|55=AAPL|54=1|10=0|",
                30 + at
            )
        };
        let lines = [
            report(0, "A", "E1"),
            report(1, "B", "E2"),
            report(2, "C", "E3"),
            report(3, "D", "E4"),
        ];
        let chain = walked(
            &codec,
            &lines.iter().map(|line| line.as_bytes()).collect::<Vec<_>>(),
        );
        assert_eq!(chain.len(), 4);
        let held: Vec<(Option<String>, Option<String>, Option<String>)> = chain
            .iter()
            .map(|message| {
                (
                    fix(message, "orderid"),
                    fix(message, "parentorderid"),
                    fix(message, "origorderid"),
                )
            })
            .collect();
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(
            held,
            [
                (some("A"), None, None),
                (some("B"), some("A"), some("A")),
                (some("C"), some("B"), some("A")),
                (some("D"), some("C"), some("A")),
            ]
        );
        // The same walk joins them into one chain, under the first's code.
        for message in &chain {
            assert_eq!(message.get_crosscode(), chain[0].get_crosscode());
        }
        // The client order identifier never moved: no parent for it.
        for message in &chain {
            assert_eq!(fix(message, "clordid").as_deref(), Some("CL1"));
            assert_eq!(fix(message, "origclordid"), None);
        }
        // Each parent is the value the walk took from the message before it,
        // under the source that stated it.
        let last = chain.last().expect("a last message");
        let sources: Vec<String> = last
            .get_identifiers()
            .of_kind(&word("parentorderid"))
            .map(|id| id.key().to_string())
            .collect();
        assert!(sources.contains(&"parentorderid".to_owned()), "{sources:?}");
    }

    /// A message that names no `OrderID(37)` carries the chain's, and the
    /// parents the chain gave it with it: a cancel request between the
    /// order's restatement and the cancel's acknowledgement leaves the
    /// acknowledgement the lineage it would have had without it.
    #[test]
    fn an_order_identifier_keeps_its_parents_across_a_message_that_states_none() {
        crate::install::installed();
        let codec = super::reader();
        let chain = walked(
            &codec,
            &[
                b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|38=10|44=100|10=0|",
                b"8=FIX.4.4|35=8|52=20260921-10:00:01|11=C1|37=O1|150=0|39=0|54=1|55=AAPL|10=0|",
                // The venue restates the order under a new OrderID.
                b"8=FIX.4.4|35=8|52=20260921-10:00:02|11=C1|37=O2|150=D|39=0|54=1|55=AAPL|10=0|",
                // A cancel request naming no OrderID.
                b"8=FIX.4.4|35=F|52=20260921-10:00:03|11=C2|41=C1|54=1|55=AAPL|38=10|10=0|",
                b"8=FIX.4.4|35=8|52=20260921-10:00:04|11=C2|41=C1|37=O2|150=4|39=4|54=1|55=AAPL|10=0|",
            ],
        );
        assert_eq!(chain.len(), 5);
        assert!(
            chain
                .windows(2)
                .all(|pair| pair[1].get_prevuuid() == Some(pair[0].get_uuid())),
            "one chain"
        );
        let lineage = |message: &FixMsg| {
            [
                fix(message, "orderid"),
                fix(message, "parentorderid"),
                fix(message, "origorderid"),
            ]
        };
        let some = |text: &str| Some(text.to_owned());
        let restated = [some("O2"), some("O1"), some("O1")];
        assert_eq!(lineage(&chain[2]), restated);
        assert_eq!(lineage(&chain[3]), restated, "carried with the base");
        assert_eq!(lineage(&chain[4]), restated, "the base kept its value");
        // A parent travels with a base the chain carries, and only then: the
        // client order identifier is not carried, nor is its parent.
        assert_eq!(fix(&chain[3], "clordid").as_deref(), Some("C2"));
        assert_eq!(fix(&chain[3], "origclordid").as_deref(), Some("C1"));
    }

    /// A registry that states its own list for `OrderID(37)` is the one the
    /// walk reads: three parents, the nearest the value before the last
    /// change, the middle ones shifting one step nearer with each change,
    /// the last the chain's first.
    #[test]
    fn a_registrys_own_list_of_parents_is_the_one_the_walk_reads() {
        crate::install::installed();
        let mut registry = (*super::super::committed_registry()).clone();
        let mut orderid = registry.field_by_tag(37).expect("OrderID(37)").clone();
        FixFieldMut::new(&mut orderid)
            .set_parents(["parentorderid", "grandparentorderid", "origorderid"])
            .expect("three types");
        registry.update(orderid).expect("the field restated");
        let codec = super::super::fixed_codec(Arc::new(registry));
        let report = |at: u32, orderid: &str, execid: &str| {
            format!(
                "8=FIX.4.4|35=8|52=20260102-10:15:{:02}|11=CL1|37={orderid}|17={execid}|150=0|39=0|55=AAPL|54=1|10=0|",
                30 + at
            )
        };
        let lines = [
            report(0, "A", "E1"),
            report(1, "B", "E2"),
            report(2, "C", "E3"),
            report(3, "D", "E4"),
        ];
        let chain = walked(
            &codec,
            &lines.iter().map(|line| line.as_bytes()).collect::<Vec<_>>(),
        );
        let held: Vec<[Option<String>; 4]> = chain
            .iter()
            .map(|message| {
                [
                    fix(message, "orderid"),
                    fix(message, "parentorderid"),
                    fix(message, "grandparentorderid"),
                    fix(message, "origorderid"),
                ]
            })
            .collect();
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(
            held,
            [
                [some("A"), None, None, None],
                // The first change has no grandparent yet: the last of the
                // list is the chain's first value, the one before it.
                [some("B"), some("A"), None, some("A")],
                [some("C"), some("B"), some("A"), some("A")],
                [some("D"), some("C"), some("B"), some("A")],
            ]
        );
    }

    /// A message stated before the chain's live one - out of order, so read
    /// from a source in instant order once its hour was walked - follows
    /// nothing, yet it is one of the chain's statements: the walk re-keys it
    /// onto the chain, the side the chain states written as `Side(54)`
    /// first, so its stored cross code is the chain's, its cross hash and
    /// cross element derive from that code, and its identity is settled
    /// once under them - its content code moved by the side it was lent.
    #[test]
    fn a_late_message_stating_no_side_is_rekeyed_onto_its_chain() {
        crate::install::installed();
        use yggdryl_market::Side;

        let codec = super::reader().with_sorted_lifecycle(true);
        let late: &[u8] = b"8=FIX.4.2|35=9|49=S|56=B|34=72|52=20260814-10:00:00.500|11=C-2|37=O-1|41=C-1|39=8|434=1|60=20260814-10:00:00.500|10=0|";
        let chain = walked(
            &codec,
            &[
                b"8=FIX.4.2|35=D|49=B|56=S|34=70|52=20260814-10:00:00|11=C-1|55=2454|54=2|38=100|40=2|44=10|60=20260814-10:00:00|10=0|",
                b"8=FIX.4.2|35=8|49=S|56=B|34=71|52=20260814-10:00:01|11=C-1|37=O-1|17=X1|150=0|39=0|54=2|55=2454|38=100|44=10|151=100|14=0|6=0|60=20260814-10:00:01|10=0|",
                // Another order three hours on: the hour of the first two is
                // walked before the late reject is read.
                b"8=FIX.4.2|35=D|49=B|56=S|34=73|52=20260814-13:30:00|11=Z-1|55=2330|54=1|38=100|40=2|44=10|60=20260814-13:30:00|10=0|",
                late,
            ],
        );
        let of_type = |msgtype: &str| {
            chain
                .iter()
                .find(|message| message.header().msgtype() == msgtype)
                .unwrap_or_else(|| panic!("a {msgtype} in {}", chain.len()))
        };
        let (order, ack, stray) = (of_type("D"), of_type("8"), of_type("9"));
        assert_eq!(chain.len(), 4);
        assert_eq!(ack.get_prevuuid(), Some(order.get_uuid()));
        let parsed = codec.sole_line(late).expect("the late line");
        assert_eq!(parsed.get_side(), Side::Unknown);
        assert_eq!(parsed.get_crosscode(), "10:0:O-1", "its own code, no side");
        assert!(stray.is_before(ack), "stated before the live one");
        assert_eq!(stray.get_prevuuid(), None, "it follows nothing");
        assert_eq!(stray.get_side(), Side::Sell);
        assert_eq!(super::text(stray, 54).as_deref(), Some("SELL"));
        assert_eq!(stray.get_crosscode(), "10:2:C-1");
        assert_eq!(stray.get_crosscode(), order.get_crosscode());
        assert_eq!(stray.get_crosshashcode(), order.get_crosshashcode());
        assert_eq!(stray.get_crossuuid(), order.get_crossuuid());
        assert_eq!(
            stray.get_crossuuid(),
            yggdryl::Uuid::from_v8(u128::from(stray.get_crosshashcode()))
        );
        assert_ne!(
            stray.get_hashcode(),
            parsed.get_hashcode(),
            "the side it was lent is content"
        );
        assert_eq!(stray.get_uuid(), stray.time_uuid().expect("an identity"));
        let wire = String::from_utf8(stray.into_bytes(b'|')).expect("a text wire");
        assert!(wire.contains("|54=2|"), "{wire}");
        // The live statement moved not at all.
        assert_eq!(ack.get_crosscode(), order.get_crosscode());
    }

    /// A report citing two live chains - the `ClOrdID(11)` one chain holds,
    /// the `OrderID(37)` the other's code is alive under - is a conflict the
    /// walk states rather than picks: the report stands under its own
    /// identity, the chain its own code is alive under, and carries the
    /// conflict as a `FixAnomaly` under `crosscode` naming both chains'
    /// codes; the report delivered again under another `MsgSeqNum(34)` - one
    /// content, one identity - resolves through the same conflict and
    /// carries the same anomaly once.
    #[test]
    fn a_message_citing_two_live_chains_carries_the_conflict_and_so_does_its_twin() {
        crate::install::installed();
        let codec = super::reader().with_dedup_window_ms(0);
        let chain = walked(
            &codec,
            &[
                b"8=FIX.4.4|35=D|49=B|56=S|34=1|52=20260921-10:00:00|11=C1|55=AAPL|54=1|38=10|44=100|10=0|",
                // The venue's report of an order the walk has not seen named
                // by its OrderID: a chain of its own.
                b"8=FIX.4.4|35=8|49=S|56=B|34=2|52=20260921-10:00:01|37=O1|17=E1|150=0|39=0|54=1|55=AAPL|10=0|",
                b"8=FIX.4.4|35=8|49=S|56=B|34=3|52=20260921-10:00:02|11=C1|37=O1|17=E2|150=0|39=0|54=1|55=AAPL|10=0|",
                b"8=FIX.4.4|35=8|49=S|56=B|34=4|52=20260921-10:00:02|11=C1|37=O1|17=E2|150=0|39=0|54=1|55=AAPL|10=0|",
            ],
        );
        let [order, venue, report, twin] = chain.as_slice() else {
            panic!("four statements, not {}", chain.len())
        };
        assert_eq!(order.get_crosscode(), "10:1:C1");
        assert_eq!(venue.get_crosscode(), "10:1:O1");
        assert_eq!(venue.get_prevuuid(), None, "two chains");
        assert_eq!(report.get_prevuuid(), Some(venue.get_uuid()), "its own");
        assert_eq!(report.get_crosscode(), venue.get_crosscode());
        let conflicts: Vec<&yggdryl_fix::FixAnomaly> = report
            .anomalies()
            .iter()
            .filter(|anomaly| anomaly.field() == "crosscode")
            .collect();
        let [conflict] = conflicts.as_slice() else {
            panic!("one conflict, not {conflicts:?}")
        };
        for named in ["10:1:O1", "10:1:C1", "clordid=C1"] {
            assert!(conflict.reason().contains(named), "{named} in {conflict}");
        }
        assert_eq!(twin.get_uuid(), report.get_uuid());
        assert_eq!(twin.anomalies(), report.anomalies());
        assert_eq!(twin.get_crosscode(), report.get_crosscode());
        // A message citing one chain only states no conflict.
        assert!(
            order
                .anomalies()
                .iter()
                .chain(venue.anomalies())
                .all(|anomaly| anomaly.field() != "crosscode")
        );
    }
}

/// An order canceled, expired or rejected left nothing whatever it traded,
/// so `OrderQty(38)` less a `LeavesQty(151)` of nothing is no `CumQty(14)`:
/// what traded is read only where the report states what was canceled
/// beside them - `OrderQty - LeavesQty - CxlQty` - or the order still works
/// or filled, `OrderQty - LeavesQty`.
#[test]
fn a_canceled_report_derives_no_cumulative_quantity_from_its_order() {
    crate::install::installed();
    let reader = reader();
    for status in ["4", "C", "8", "3"] {
        let line = format!("8=FIX.4.4|35=8|37=A|39={status}|38=100|10=0|");
        let closed = settled(&reader, line.as_bytes());
        assert_eq!(closed.by_tag(151).unwrap(), super::decimal("0"), "{status}");
        assert_eq!(closed.get_by_tag(14), None, "39={status}");
        assert_eq!(closed.get_cumqty(), None, "39={status}");
    }
    let canceled = settled(&reader, b"8=FIX.4.4|35=8|37=A|39=4|38=100|84=60|10=0|");
    assert_eq!(canceled.by_tag(14).unwrap(), super::decimal("40"));
    assert_eq!(canceled.get_cxlqty(), Some("60".parse().unwrap()));
    // Working or filled, what is left is what was ordered less what traded.
    let working = settled(&reader, b"8=FIX.4.4|35=8|37=A|39=1|38=100|151=60|10=0|");
    assert_eq!(working.by_tag(14).unwrap(), super::decimal("40"));
    let filled = settled(&reader, b"8=FIX.4.4|35=8|37=A|39=2|38=100|10=0|");
    assert_eq!(filled.by_tag(14).unwrap(), super::decimal("100"));
}

/// An order that ended any way but filled left nothing whatever it ordered,
/// so the `LeavesQty(151)` of nothing a report's status implies says nothing
/// of what was ordered: `OrderQty(38)` is what traded plus what was canceled
/// there, and nothing where the report states no `CxlQty(84)`.
#[test]
fn an_ended_report_derives_no_order_quantity_from_what_it_left() {
    crate::install::installed();
    let reader = reader();
    for (line, expected) in [
        ("8=FIX.4.4|35=8|37=A|150=8|39=8|14=0|10=0|", None),
        ("8=FIX.4.4|35=8|37=A|150=4|39=4|14=40|10=0|", None),
        ("8=FIX.4.4|35=8|37=A|150=3|39=3|14=40|10=0|", None),
        (
            "8=FIX.4.4|35=8|37=A|150=4|39=4|14=40|84=60|10=0|",
            Some("100"),
        ),
        // Filled or still working, what it ordered is what traded plus what
        // is left.
        ("8=FIX.4.4|35=8|37=A|150=F|39=2|14=40|10=0|", Some("40")),
        (
            "8=FIX.4.4|35=8|37=A|150=F|39=1|14=40|151=60|10=0|",
            Some("100"),
        ),
    ] {
        let report = settled(&reader, line.as_bytes());
        assert_eq!(
            report.get_by_tag(38),
            expected.map(super::decimal),
            "{line}"
        );
    }
}

/// An order asked for and not yet acknowledged, or accepted for bidding,
/// still works: what it has left is all it ordered less what traded - as
/// FIX states for every `OrdStatus(39)` but the ones that ended it, which
/// leave nothing.
#[test]
fn a_pending_new_report_leaves_all_it_ordered() {
    crate::install::installed();
    let reader = reader();
    for status in ["A", "D", "E", "6", "9"] {
        let line = format!("8=FIX.4.4|35=8|37=A|39={status}|38=100|14=0|10=0|");
        let live = settled(&reader, line.as_bytes());
        assert_eq!(live.by_tag(151).unwrap(), super::decimal("100"), "{status}");
        assert_eq!(live.get_leavesqty(), Some("100".parse().unwrap()));
    }
    let calculated = settled(&reader, b"8=FIX.4.4|35=8|37=A|39=B|38=100|14=100|10=0|");
    assert_eq!(calculated.by_tag(151).unwrap(), super::decimal("0"));
    // Stating nothing traded, all it ordered is left all the same.
    let pending = settled(&reader, b"8=FIX.4.4|35=8|37=A|39=A|38=100|10=0|");
    assert_eq!(pending.get_leavesqty(), Some("100".parse().unwrap()));
}

/// A fill's report and the execution split off it each state a quantity of
/// their own: the report its order's - what it has left open - and the
/// execution the fill it reports, its `lastqty`, `LastQty(32)` (decision 27:
/// one definition of the quantity per kind, available on an order, executed
/// on an execution), quoted on the side it took and never what the order
/// has left.
#[test]
fn a_fill_split_report_keeps_its_leaves_as_its_quantity() {
    crate::install::installed();
    use yggdryl_market::MarketDataKind;

    let messages: Vec<FixMsg> = reader()
        .parse_line(
            b"8=FIX.4.4|35=8|52=20260921-10:00:00|17=E-1|37=O-9|11=C-9|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|10=0|",
        )
        .expect("a readable line")
        .collect::<yggdryl::Result<_>>()
        .expect("the report and its fill");
    let [report, execution] = messages.as_slice() else {
        panic!("the report and its execution, got {}", messages.len())
    };
    assert_eq!(report.marketdatakind(), MarketDataKind::Order);
    assert_eq!(report.get_leavesqty(), Some("60".parse().unwrap()));
    assert_eq!(report.get_quantity(), Some("60".parse().unwrap()));
    assert_eq!(report.get_bidqty(), Some("60".parse().unwrap()));
    assert_eq!(execution.marketdatakind(), MarketDataKind::Execution);
    assert_eq!(execution.get_quantity(), Some("40".parse().unwrap()));
    assert_eq!(execution.get_bidqty(), Some("40".parse().unwrap()));
    assert_eq!(execution.get_leavesqty(), Some("60".parse().unwrap()));
    assert_eq!(execution.get_lastqty(), Some("40".parse().unwrap()));

    // A filled order's report has nothing left; its execution the fill.
    let messages: Vec<FixMsg> = reader()
        .parse_line(
            b"8=FIX.4.4|35=8|17=E-2|37=O-9|39=2|150=F|55=AAPL|54=2|38=100|14=100|32=60|31=10|10=0|",
        )
        .expect("a readable line")
        .collect::<yggdryl::Result<_>>()
        .expect("the report and its fill");
    assert_eq!(messages[0].get_quantity(), Some("0".parse().unwrap()));
    assert_eq!(messages[1].get_quantity(), Some("60".parse().unwrap()));
    assert_eq!(messages[1].get_askqty(), Some("60".parse().unwrap()));
    assert_eq!(messages[1].get_lastqty(), Some("60".parse().unwrap()));
}

/// A dictionary that leaves each tag text - one tagged `utf8` field apiece -
/// so what crosses it is the spelling the wire carried.
fn text_registry(tags: &[(&str, i32)]) -> Arc<FixRegistry> {
    let fields = tags.iter().map(|&(name, tag)| {
        let mut field = yggdryl::DataType::utf8().nullable_field(name);
        FixFieldMut::new(&mut field).set_tag(tag).expect("a tag");
        field
    });
    Arc::new(FixRegistry::from_fields(fields).expect("a dictionary"))
}

#[test]
fn the_native_rules_read_a_code_a_dictionary_left_as_text_as_the_number_it_spells() {
    crate::install::installed();
    // `Product(460)` and `PutOrCall(201)` are numbers in the committed
    // dictionary; one that leaves them text states what the wire spelled,
    // which the one integer reader reads as it reads a number.
    let reader = super::fixed_codec(text_registry(&[
        ("currency", 15),
        ("settlcurrency", 120),
        ("product", 460),
        ("securitytype", 167),
        ("putorcall", 201),
        ("cficode", 461),
    ]));
    for (body, tag, expected) in [
        ("460=4|15=EUR|", 120, None),
        ("460=1|15=EUR|", 120, Some("EUR")),
        ("460= 4 |120=USD|", 15, None),
        ("167=OPT|201=1|", 461, Some("OCXXXX")),
        ("167=OPT|201=0|", 461, Some("OPXXXX")),
        ("167=OPT|201=abc|", 461, Some("OXXXXX")),
    ] {
        let line = format!("8=FIX.4.4|35=D|{body}10=0|");
        let held = reader.sole_line(line.as_bytes()).expect("a readable line");
        assert_eq!(text(&held, tag).as_deref(), expected, "{tag} of {line}");
    }
}

#[test]
fn a_resend_flag_a_dictionary_left_as_text_marks_a_replay_as_a_boolean_does() {
    crate::install::installed();
    // The committed dictionary types `PossResend(97)` a boolean; a dictionary
    // that leaves the tag text states the spelling the wire carried, which
    // the one boolean reader reads: the retransmission a second later is the
    // same delivery, dated by the time it names, for every spelling of true
    // and for none other. A header field the committed dictionary references
    // is neither removed nor retyped, so that dictionary is built beside it:
    // every other field under the type the committed one holds, `97` text.
    let committed = super::committed_registry();
    let fields = [8, 35, 49, 56, 34, 52, 122, 11, 55, 54, 38, 10]
        .into_iter()
        .map(|tag| {
            let held = committed.field_by_tag(tag).expect("a committed field");
            let mut field = held.dtype().clone().nullable_field(held.name());
            FixFieldMut::new(&mut field).set_tag(tag).expect("a tag");
            field
        })
        .chain(std::iter::once({
            let mut field = yggdryl::DataType::utf8().nullable_field("possresend");
            FixFieldMut::new(&mut field).set_tag(97).expect("a tag");
            field
        }));
    let registry = FixRegistry::from_fields(fields).expect("a dictionary");
    let codec = super::fixed_codec(Arc::new(registry));
    let parse = |line: &str| codec.parse_fix_line(line.as_bytes()).expect("a message");
    let walked = |flag: &str| -> usize {
        let messages = vec![
            parse(
                "8=FIX.4.4|35=D|49=S|56=T|34=7|52=20260102-10:15:30|11=A|55=AAPL|54=1|38=100|10=0|",
            ),
            parse(&format!(
                "8=FIX.4.4|35=D|49=S|56=T|34=7|97={flag}|52=20260102-10:15:31|122=20260102-10:15:30|11=A|55=AAPL|54=1|38=100|10=0|"
            )),
        ];
        codec
            .lifecycle(messages)
            .collect::<yggdryl::Result<Vec<FixMsg>>>()
            .expect("the walk")
            .len()
    };
    for flag in ["Y", "yes", "1", "on", "true"] {
        assert_eq!(walked(flag), 1, "97={flag} is a replay");
    }
    for flag in ["N", "no", "0", "maybe"] {
        assert_eq!(walked(flag), 2, "97={flag} is no replay");
    }
}

/// A walk learns the instrument a message's own is written on - its
/// underlying - off the wire's `UnderlyingInstrument`, a related `Underlier`
/// or a bridge's key, held as the cross code of the instrument the
/// collection keys by that ISIN - one it keys none by names no underlying -
/// and lifts it nowhere: the message's security identifiers stay its own,
/// and the wire is the parse's.
#[test]
fn a_walk_learns_the_underlying_a_message_names_and_lifts_it_nowhere() {
    crate::install::installed();
    use std::sync::{Arc, Mutex};
    use yggdryl_market::Instruments;

    let instruments = Arc::new(Mutex::new(Instruments::new()));
    let codec =
        super::fixed_codec(super::committed_registry()).with_instruments(Arc::clone(&instruments));
    let line = |seq: i32, body: &str| {
        format!(
            "8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|{body}|10=0|"
        )
    };
    let walk = |body: &str, seq: i32| -> Vec<FixMsg> {
        let parsed: Vec<FixMsg> = codec
            .parse_lines([line(seq, body)])
            .collect::<yggdryl::Result<_>>()
            .expect("a message");
        codec
            .lifecycle(parsed)
            .collect::<yggdryl::Result<_>>()
            .expect("a walk")
    };
    let warrant = {
        let body = "CH000000000";
        format!("{body}{}", Isin::closing_digit(body).unwrap())
    };
    let underlying = || {
        instruments
            .lock()
            .expect("the registry")
            .get(&warrant)
            .and_then(|row| row.underlying().map(str::to_owned))
    };
    // An underlying no instrument keys names none.
    walk(
        &format!("22=4|48={warrant}|711=1|311=HOLN|309=CH0012214059|305=4"),
        20,
    );
    assert_eq!(underlying(), None);
    // The securities the underlyings below name, learned first.
    for (seq, isin) in [
        (21, "CH0012214059"),
        (22, "US0378331005"),
        (23, "CH0012005267"),
    ] {
        walk(&format!("22=4|48={isin}"), seq);
    }
    // The wire's `UnderlyingInstrument` in a `NoUnderlyings(711)` occurrence
    // is learned and lifted nowhere.
    let walked = walk(
        &format!("22=4|48={warrant}|711=1|311=HOLN|309=CH0012214059|305=4"),
        1,
    );
    assert_eq!(underlying().as_deref(), Some("CH0012214059"));
    let message = &walked[0];
    assert_eq!(message.get_isincode(), Some(warrant.as_str()));
    let stated: Vec<String> = message
        .get_securityids()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        stated.iter().all(|id| !id.contains("CH0012214059")),
        "lifted nowhere: {stated:?}"
    );
    assert!(
        message
            .into_bytes(b'|')
            .windows(17)
            .any(|w| w == b"309=CH0012214059|"),
        "the wire is the parse's"
    );
    // A bridge key naming an underlying's ISIN replaces it.
    walk(&format!("22=4|48={warrant}|UnderlyingISIN=US0378331005"), 2);
    assert_eq!(underlying().as_deref(), Some("US0378331005"));
    // An `UnderlyingSymbol(311)` shaped as an ISIN alone.
    walk(&format!("22=4|48={warrant}|711=1|311=CH0012005267"), 3);
    assert_eq!(underlying().as_deref(), Some("CH0012005267"));
    // A related instrument typed `Underlier`; one typed otherwise states
    // nothing.
    walk(
        &format!("22=4|48={warrant}|1647=1|1648=2|1650=US0378331005|1651=4"),
        4,
    );
    assert_eq!(underlying().as_deref(), Some("US0378331005"));
    walk(
        &format!("22=4|48={warrant}|1647=1|1648=6|1650=CH0012005267|1651=4"),
        5,
    );
    assert_eq!(underlying().as_deref(), Some("US0378331005"));
    // A basket, the message's own ISIN, another source's code and a number
    // that does not close each state nothing.
    let unlearned = [
        "711=2|311=A|309=CH0012214059|305=4|311=B|309=CH0012005267|305=4".to_owned(),
        format!("309={warrant}|305=4"),
        "309=037833100|305=1".to_owned(),
        "309=CH0012214058|305=4".to_owned(),
    ];
    for (at, body) in unlearned.iter().enumerate() {
        walk(&format!("22=4|48={warrant}|{body}"), 6 + at as i32);
        assert_eq!(underlying().as_deref(), Some("US0378331005"), "{body}");
    }
    // A message stating no real ISIN of its own learns nothing.
    walk("55=HOLN|711=1|311=HOLN|309=CH0012214059|305=4", 11);
    assert_eq!(instruments.lock().expect("the registry").len(), 4);
}

/// A walk learns the EUSIPA product category a bridge's key states beside
/// the message's ISIN - `EUSIPACode`, `OMS_SSPACategory`, SIX's
/// `X-SWX-SSPA` - and lifts it nowhere: the key stays among the message's
/// entries and its leaf's metadata, no identifier holds it, and a later
/// message of the instrument is filled with nothing of it. Two categories,
/// text of no category's shape, a key naming the category's name, a key
/// naming another instrument's category - an underlying's, a leg's, a
/// contra's, a related or a benchmark instrument's, by the words a security
/// type is refused by - and a message stating no real ISIN of its own each
/// learn none.
#[test]
fn a_walk_learns_the_product_category_a_bridge_key_states_and_lifts_it_nowhere() {
    crate::install::installed();
    use std::sync::{Arc, Mutex};
    use yggdryl_market::Instruments;
    use yggdryl_market::graph::{Market, Operation};

    let instruments = Arc::new(Mutex::new(Instruments::new()));
    let codec =
        super::fixed_codec(super::committed_registry()).with_instruments(Arc::clone(&instruments));
    let line = |seq: i32, body: &str| {
        format!(
            "8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|{body}|10=0|"
        )
    };
    let walk = |body: &str, seq: i32| -> Vec<FixMsg> {
        let parsed: Vec<FixMsg> = codec
            .parse_lines([line(seq, body)])
            .collect::<yggdryl::Result<_>>()
            .expect("a message");
        codec
            .lifecycle(parsed)
            .collect::<yggdryl::Result<_>>()
            .expect("a walk")
    };
    let mini = {
        let body = "CH000000000";
        format!("{body}{}", Isin::closing_digit(body).unwrap())
    };
    let category = || {
        instruments
            .lock()
            .expect("the registry")
            .get(&mini)
            .and_then(|row| row.eusipacode())
            .map(|code| code.code())
    };
    let walked = walk(&format!("22=4|48={mini}|EUSIPACode=2300"), 1);
    assert_eq!(category(), Some(2300));
    let message = &walked[0];
    assert!(
        message
            .get_securityids()
            .iter()
            .chain(message.get_identifiers().iter())
            .all(|id| id.value() != "2300"),
        "lifted nowhere"
    );
    let wire = String::from_utf8(message.clone().into_bytes(b'|')).expect("a text wire");
    assert!(
        wire.to_ascii_lowercase().contains("|eusipacode=2300|"),
        "the entry stays on the wire: {wire}"
    );
    // Either map's name, a namespace before it, replaces it.
    walk(&format!("22=4|48={mini}|OMS_SSPACategory=1260"), 2);
    assert_eq!(category(), Some(1260));
    walk(&format!("22=4|48={mini}|X-SWX-SSPA= 2205 "), 3);
    assert_eq!(category(), Some(2205));
    // A category neither map lists is a category.
    walk(&format!("22=4|48={mini}|firm.x.EUSIPA=2301"), 4);
    assert_eq!(category(), Some(2301));
    let unlearned = [
        "EUSIPACode=2300|SSPACategory=1260",
        "EUSIPACode=3100",
        "EUSIPACode=Mini-Future",
        "EUSIPA_Name=2300",
    ];
    for (at, body) in unlearned.iter().enumerate() {
        walk(&format!("22=4|48={mini}|{body}"), 5 + at as i32);
        assert_eq!(category(), Some(2301), "{body}");
    }
    // Another instrument's category - opening the key or spelled just
    // before the category word, after any namespace - is never this one's.
    let another = [
        "UnderlyingEUSIPA=2300",
        "LegSSPACategory=2300",
        "ContraEUSIPA=2300",
        "RelatedSSPA=2300",
        "BenchmarkEUSIPACode=2300",
        "OMS_UnderlyingEUSIPACode=2300",
        "firm.x.LegSSPA=2300",
    ];
    for (at, body) in another.iter().enumerate() {
        walk(&format!("22=4|48={mini}|{body}"), 20 + at as i32);
        assert_eq!(category(), Some(2301), "{body}");
    }
    // Beside them, this instrument's own still learns.
    walk(
        &format!("22=4|48={mini}|UnderlyingEUSIPA=2300|OMS_EUSIPACode=1260"),
        30,
    );
    assert_eq!(category(), Some(1260));
    walk(&format!("22=4|48={mini}|firm.x.EUSIPA=2301"), 31);
    assert_eq!(category(), Some(2301));
    // The same category twice is one statement.
    walk(&format!("22=4|48={mini}|EUSIPACode=2300|SSPA=2300"), 9);
    assert_eq!(category(), Some(2300));
    // A message stating no real ISIN of its own learns nothing.
    walk("55=HOLN|EUSIPACode=2300", 10);
    assert_eq!(instruments.lock().expect("the registry").len(), 1);
    // `learn` alone states none: the reading is the lifecycle's.
    let mut fresh = Instruments::new();
    let parsed: Vec<FixMsg> = codec
        .parse_lines([line(
            11,
            &format!("22=4|48={mini}|55=MINI|207=XSWX|EUSIPACode=2300"),
        )])
        .collect::<yggdryl::Result<_>>()
        .expect("a message");
    assert!(fresh.learn(&parsed[0]), "the ticker and the market");
    assert_eq!(fresh.get(&mini).and_then(|row| row.eusipacode()), None);
}

/// A walk learns a message's country of issue - where it states one its
/// ISIN does not already say - and its trading currency beside the ticker
/// and the market, and fills them into a later message of the instrument
/// where it leaves them unsaid, settled no further than the market facts
/// they imply: the message's identity and its wire are the parse's.
/// The lifecycle takes the economic match - an instrument found by the
/// short name a message states in its currency, its ISIN derived - only
/// where the registry states it, and a parse door never does: the parse
/// takes the exact tier alone. Within one walk every message stating the
/// same short name and currency takes the one answer.
#[test]
fn a_walk_takes_the_economic_match_only_where_the_registry_states_it() {
    crate::install::installed();
    use std::sync::{Arc, Mutex};
    use yggdryl::Mic;
    use yggdryl_market::{Instrument, Instruments, Listing};

    const APPLE: &str = "US0378331005";
    let mut registry = Instruments::new();
    registry
        .merge(
            Instrument::for_security(Isin::new(APPLE).unwrap())
                .unwrap()
                .with_listing(Listing::new(Some(Mic::new("XNAS").unwrap())))
                .unwrap()
                .with_fisn(Some(yggdryl::Fisn::new("APPLE INC./SH").unwrap())),
        )
        .unwrap();
    let instruments = Arc::new(Mutex::new(registry));
    let codec =
        super::fixed_codec(super::committed_registry()).with_instruments(Arc::clone(&instruments));
    let lines = |first: i32| {
        (first..first + 2).map(|seq| {
            format!(
                "8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|2737=APPLE INC/SH|15=USD|10=0|"
            )
        })
    };
    let walk = |first: i32| -> Vec<FixMsg> {
        let parsed: Vec<FixMsg> = codec
            .parse_lines(lines(first))
            .collect::<yggdryl::Result<_>>()
            .expect("two messages");
        for message in &parsed {
            assert_eq!(isincode(message), None, "a parse takes no economic match");
        }
        codec
            .lifecycle(parsed)
            .collect::<yggdryl::Result<_>>()
            .expect("a walk")
    };
    let walked = walk(1);
    assert_eq!(walked.len(), 2);
    assert!(
        walked.iter().all(|message| isincode(message).is_none()),
        "off by default"
    );
    instruments
        .lock()
        .expect("the registry")
        .set_economic_match(true);
    let walked = walk(3);
    assert_eq!(walked.len(), 2);
    for message in &walked {
        assert_eq!(isincode(message).as_deref(), Some(APPLE));
        assert!(message.get_securityids().is_derived(&IdType::Isin));
    }
    let parsed = codec
        .parse_fix_line(lines(5).next().unwrap().as_bytes())
        .expect("a message");
    assert_eq!(
        isincode(&parsed),
        None,
        "enabled, and still no parse takes it"
    );
}

#[test]
fn a_walk_learns_the_country_and_the_currency_and_fills_them_unsettled() {
    crate::install::installed();
    use std::sync::{Arc, Mutex};
    use yggdryl::Country;
    use yggdryl::graph::Element;
    use yggdryl_market::Instruments;

    let instruments = Arc::new(Mutex::new(Instruments::new()));
    let codec =
        super::fixed_codec(super::committed_registry()).with_instruments(Arc::clone(&instruments));
    let line = |seq: i32, body: &str| {
        format!(
            "8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|{body}|10=0|"
        )
    };
    let walk = |body: &str, seq: i32| -> Vec<FixMsg> {
        let parsed: Vec<FixMsg> = codec
            .parse_lines([line(seq, body)])
            .collect::<yggdryl::Result<_>>()
            .expect("a message");
        codec
            .lifecycle(parsed)
            .collect::<yggdryl::Result<_>>()
            .expect("a walk")
    };
    // A country the prefix already says states nothing; another is learned,
    // with the currency of the listing.
    walk("22=4|48=CH0012214059|470=CH|15=CHF|55=HOLN|207=XSWX", 1);
    {
        let held = instruments.lock().expect("the registry");
        let row = held.get("CH0012214059").expect("learned");
        let xswx = yggdryl::Mic::new("XSWX").unwrap();
        assert_eq!(row.countrycode(), None);
        assert_eq!(row.country(), Some(Country::new("CH").unwrap()));
        assert_eq!(
            row.listing(Some(&xswx))
                .and_then(yggdryl_market::Listing::currency)
                .map(|code| code.as_str()),
            Some("CHF"),
            "the trading currency is the listing's"
        );
        assert_eq!(row.ticker(Some(&xswx)), Some("HOLN"));
        assert_eq!(
            row.listings()
                .iter()
                .map(|listing| listing.miccode().map(|code| code.as_str()))
                .collect::<Vec<_>>(),
            [Some("XSWX")]
        );
    }
    let countrycode = || {
        instruments
            .lock()
            .expect("the registry")
            .get("CH0012214059")
            .and_then(|row| row.countrycode().map(|code| code.as_str().to_owned()))
    };
    walk("22=4|48=CH0012214059|470=LI", 2);
    assert_eq!(countrycode(), Some("LI".to_owned()));
    // The prefix states nothing a walk learns: the rule lands it on every
    // message stating no country, so neither a message stating none nor
    // one stating the prefix itself takes the held country back.
    walk("22=4|48=CH0012214059", 7);
    assert_eq!(countrycode(), Some("LI".to_owned()));
    walk("22=4|48=CH0012214059|470=CH", 8);
    assert_eq!(countrycode(), Some("LI".to_owned()));
    // An explicit merge stating the prefix does.
    instruments
        .lock()
        .expect("the registry")
        .merge(
            yggdryl_market::Instrument::for_security(yggdryl::Isin::new("CH0012214059").unwrap())
                .unwrap()
                .with_countrycode(Some(Country::new("CH").unwrap())),
        )
        .unwrap();
    assert_eq!(countrycode(), None);
    // A later message naming the ticker on the market takes the ISIN the
    // table names at the parse, derived, and the currency as a market fact
    // in the walk - the wire and the identity the parse gave it untouched.
    let parsed = codec
        .parse_fix_line(line(3, "55=HOLN|207=XSWX").as_bytes())
        .expect("a message");
    assert_eq!(parsed.get_isincode(), Some("CH0012214059"));
    assert!(parsed.get_securityids().is_derived(&IdType::Isin));
    assert!(
        parsed.get_currency().is_none(),
        "a parse fills no market fact"
    );
    let walked = walk("55=HOLN|207=XSWX", 3);
    assert_eq!(walked.len(), 1);
    let filled = &walked[0];
    assert_eq!(filled.get_isincode(), Some("CH0012214059"));
    assert!(filled.get_securityids().is_derived(&IdType::Isin));
    assert_eq!(filled.get_currency().as_str(), "CHF");
    assert_eq!(filled.get_uuid(), parsed.get_uuid());
    assert_eq!(filled.get_hashcode(), parsed.get_hashcode());
    assert_eq!(filled.into_bytes(b'|'), parsed.into_bytes(b'|'));
    assert!(!filled.into_bytes(b'|').windows(3).any(|w| w == b"15="));
    assert_eq!(filled.get_by_tag(15), None, "no field was written");

    // A pair the parse detected off the symbol keys the FX instrument -
    // `IF:EUR/USD`, its class detected too - and the real ISIN stated beside
    // it is a fact of that instrument, found by it; the dealt currency is no
    // listing's, and the instrument's currency is the pair's quote leg.
    let fx = {
        let body = "EZ000000000";
        format!("{body}{}", yggdryl::Isin::closing_digit(body).unwrap())
    };
    let walked = walk(&format!("22=4|48={fx}|55=EUR/USD|15=EUR"), 4);
    {
        let held = instruments.lock().expect("the registry");
        let row = held.get(&fx).expect("learned");
        assert_eq!(row.get_crosscode(), "IF:EUR/USD");
        assert_eq!(row.get(&IdType::Forex), Some("EUR/USD"));
        assert_eq!(row.isin(), Some(fx.as_str()), "the real number, rank two");
        assert_eq!(row.minted_isin(), None, "a numbered instrument mints none");
        assert_eq!(row.currency().map(|code| code.as_str()), Some("USD"));
        assert_eq!(
            row.listings()
                .iter()
                .map(|listing| (
                    listing.miccode().map(|code| code.as_str()),
                    listing.ticker()
                ))
                .collect::<Vec<_>>(),
            [(None, Some("EUR/USD"))],
            "the symbol is the ticker of the pair's one unlisted listing"
        );
        assert_eq!(walked[0].get_instcode(), Some("IF:EUR/USD"));
        assert_eq!(walked[0].get_isincode(), Some(fx.as_str()));
    }
    // A pair stated as the `forex` identifier alone keys the same.
    walk(&format!("22=4|48={fx}|forexcode=EUR/USD|15=EUR"), 5);
    let held = instruments.lock().expect("the registry");
    assert_eq!(held.len(), 2, "the share and the pair");
    let row = held.get("IF:EUR/USD").expect("learned");
    assert_eq!(row.get(&IdType::Forex), Some("EUR/USD"));
}

/// A walk creates the instrument of a pair a message's symbol names -
/// `IF:EUR/USD`, keyed by the class the detection settled and the pair,
/// minted its number - and every row of it carries the code the parse
/// wrote and the number as its ISIN (D42); a message stating no class the
/// pair spells, a ticker-only security, has no instrument and a null code.
#[test]
fn a_walk_creates_the_pairs_instrument_and_every_row_carries_its_code() {
    crate::install::installed();
    use std::sync::{Arc, Mutex};
    use yggdryl::graph::Element;
    use yggdryl_market::Instruments;

    let instruments = Arc::new(Mutex::new(Instruments::new()));
    let codec =
        super::fixed_codec(super::committed_registry()).with_instruments(Arc::clone(&instruments));
    let line = |seq: i32, body: &str| {
        format!(
            "8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|{body}|10=0|"
        )
    };
    let walk = |body: &str, seq: i32| -> Vec<FixMsg> {
        let parsed: Vec<FixMsg> = codec
            .parse_lines([line(seq, body)])
            .collect::<yggdryl::Result<_>>()
            .expect("a message");
        assert_eq!(
            parsed[0].get_instcode(),
            parsed[0]
                .get_securityids()
                .get(&IdType::Forex)
                .map(|_| "IF:EUR/USD"),
            "the parse writes the pair's code from the message alone: {body}"
        );
        codec
            .lifecycle(parsed)
            .collect::<yggdryl::Result<_>>()
            .expect("a walk")
    };
    let walked = walk("55=EUR/USD|54=1|38=1000000|44=1.0850", 1);
    assert_eq!(walked[0].get_instcode(), Some("IF:EUR/USD"));
    assert_eq!(walked[0].get_isincode(), Some("QYLTVIRYHNX5"));
    {
        let held = instruments.lock().expect("the registry");
        assert_eq!(held.len(), 1);
        let pair = held.get("IF:EUR/USD").expect("the pair's instrument");
        assert_eq!(pair.get_crosscode(), "IF:EUR/USD");
        assert_eq!(pair.minted_isin(), Some("QYLTVIRYHNX5"));
        assert_eq!(pair.isin(), Some("QYLTVIRYHNX5"));
        assert_eq!(pair.class(), Some("IF"));
        assert_eq!(pair.currency().map(|code| code.as_str()), Some("USD"));
        assert_eq!(pair.countrycode(), None);
        assert_eq!(
            held.get("QYLTVIRYHNX5"),
            Some(pair),
            "the number leads to it"
        );
    }
    // Another spelling of the pair is the same instrument.
    walk("55=EURUSD CURNCY|54=2|38=500000", 2);
    assert_eq!(instruments.lock().expect("the registry").len(), 1);
    // A ticker-only security has no instrument and no code.
    let walked = walk("55=AAPL|54=1|38=100", 3);
    assert_eq!(walked[0].get_instcode(), None);
    assert_eq!(instruments.lock().expect("the registry").len(), 1);
}

/// Decision 10: a walk learns a message's instrument description into the
/// instrument's metadata - the Instrument component's descriptive fields no
/// typed fact holds, under the dictionary's names, trimmed - and keeps a
/// bridge's own identifier of it under its `src:type` key beside the base
/// key it fills; the message's own metadata, a bridge's unmapped keys about
/// the order, never reaches the instrument.
#[test]
fn a_walk_learns_the_instruments_description_and_its_sourced_identifiers() {
    crate::install::installed();
    use smol_str::SmolStr;
    use std::sync::{Arc, Mutex};
    use yggdryl_market::{IdSource, Instruments};
    let instruments = Arc::new(Mutex::new(Instruments::new()));
    let codec =
        super::fixed_codec(super::committed_registry()).with_instruments(Arc::clone(&instruments));
    let line = "8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:01|11=1|48=US0378331005|22=4|55=AAPL|207=XNAS|15=USD|54=1|38=100|106= Apple Inc. |107=APPLE INC COMMON STOCK|167=CS|Bloomberg_FIGI=BBG000B9XRY4|OMS_UserID=u7|10=0|";
    let parsed: Vec<FixMsg> = codec
        .parse_lines([line.to_owned()])
        .collect::<yggdryl::Result<_>>()
        .expect("a message");
    let walked: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<_>>()
        .expect("a walk");
    assert_eq!(walked[0].get_instcode(), Some("US0378331005"));
    let held = instruments.lock().expect("the collection");
    let apple = held.get("US0378331005").expect("the security");
    let meta = |key: &str| apple.metadata().get(key).map(SmolStr::as_str);
    assert_eq!(meta("issuer"), Some("Apple Inc."), "trimmed");
    assert_eq!(meta("securitydesc"), Some("APPLE INC COMMON STOCK"));
    assert_eq!(meta("securitytype"), Some("CS"));
    assert_eq!(
        meta("product"),
        Some("5"),
        "the product the native plan implies off the security type, as its digits"
    );
    assert_eq!(
        apple.metadata().len(),
        4,
        "the order's own keys stay on the order"
    );
    let bloomberg = IdKey::new("bloomberg".parse::<IdSource>().unwrap(), IdType::Figi);
    let xnas = yggdryl::Mic::new("XNAS").unwrap();
    assert_eq!(
        apple
            .listing(Some(&xnas))
            .and_then(|listing| listing.codes().get_from(&bloomberg)),
        Some("BBG000B9XRY4"),
        "the source's own key, on the listing"
    );
    assert_eq!(apple.get(&IdType::Figi), Some("BBG000B9XRY4"));
}

/// A walk learns a derivative from what only its message spells
/// (`FixMsg::stated_characteristics`): an option by its underlying's code,
/// its expiry day and its strike, a future by its underlying and its
/// contract month, a strategy by its legs - each keyed `class:body` once the
/// underlying or the legs resolve to instruments the collection keys, its
/// real ISIN a fact beside the key - and fills the code onto the row, where
/// the parse wrote none: a derivative's code is no function of its message
/// alone (D42).
#[test]
fn a_walk_learns_a_derivative_from_what_its_message_spells() {
    crate::install::installed();
    use std::sync::{Arc, Mutex};
    use yggdryl::Decimal;
    use yggdryl::graph::Element;
    use yggdryl_market::{Exercise, Instruments};

    let instruments = Arc::new(Mutex::new(Instruments::new()));
    let codec =
        super::fixed_codec(super::committed_registry()).with_instruments(Arc::clone(&instruments));
    let line = |seq: i32, body: &str| {
        format!(
            "8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|{body}|10=0|"
        )
    };
    let walk = |body: &str, seq: i32| -> Vec<FixMsg> {
        let parsed: Vec<FixMsg> = codec
            .parse_lines([line(seq, body)])
            .collect::<yggdryl::Result<_>>()
            .expect("a message");
        codec
            .lifecycle(parsed)
            .collect::<yggdryl::Result<_>>()
            .expect("a walk")
    };
    let code = |key: &str| {
        instruments
            .lock()
            .expect("the registry")
            .get(key)
            .map(|row| row.get_crosscode().to_owned())
    };
    // The underlyings: Apple and the EURO STOXX 50 index.
    walk("22=4|48=US0378331005|461=ESVUFR", 1);
    walk("22=4|48=EU0009658145|461=TIXXXX", 2);
    // The call on Apple, Eurex-numbered, American, multiplier 100.
    let call = "DE000C000000";
    let parsed = codec
        .parse_fix_line(
            line(
                3,
                &format!(
                    "22=4|48={call}|461=OCASPS|201=1|202=200|541=20261218|1194=1|231=100|711=1|309=US0378331005|305=4"
                ),
            )
            .as_bytes(),
        )
        .expect("a message");
    assert_eq!(
        parsed.get_instcode(),
        None,
        "a derivative's code needs the table"
    );
    let (class, pair, characteristics) = parsed.stated_characteristics().expect("a class");
    assert_eq!(class.as_str(), "OCASPS");
    assert_eq!(pair, None);
    assert_eq!(
        characteristics.expiry().map(|expiry| expiry.to_string()),
        Some("2026-12-18".to_owned())
    );
    assert_eq!(
        characteristics.strikepx(),
        Some(Decimal::parse("200").unwrap())
    );
    assert_eq!(
        characteristics.multiplier(),
        Some(Decimal::parse("100").unwrap())
    );
    assert_eq!(characteristics.exercise(), Some(Exercise::American));
    assert_eq!(characteristics.settle(), None);
    let walked = codec
        .lifecycle([Ok(parsed)])
        .collect::<yggdryl::Result<Vec<FixMsg>>>()
        .expect("a walk");
    assert_eq!(
        walked[0].get_instcode(),
        Some("OC:US0378331005:2026-12-18:200"),
        "filled by the lifecycle"
    );
    assert_eq!(
        walked[0].get_isincode(),
        Some(call),
        "the real number stays the row's"
    );
    assert_eq!(
        code(call).as_deref(),
        Some("OC:US0378331005:2026-12-18:200"),
        "the number leads to the option"
    );
    {
        let held = instruments.lock().expect("the registry");
        let option = held.get(call).expect("the option");
        assert_eq!(option.isin(), Some(call));
        assert_eq!(
            option.minted_isin(),
            None,
            "a numbered instrument mints none"
        );
        assert_eq!(option.underlying(), Some("US0378331005"));
        assert_eq!(
            option.characteristics().exercise(),
            Some(Exercise::American)
        );
        assert_eq!(
            option.characteristics().multiplier(),
            Some(Decimal::parse("100").unwrap())
        );
        assert!(!option.is_placeholder());
    }
    // The December 2026 FESX future, by its contract month.
    let future = "DE000F000007";
    let walked = walk(
        &format!("22=4|48={future}|461=FFICSX|200=202612|711=1|309=EU0009658145|305=4"),
        4,
    );
    assert_eq!(walked[0].get_instcode(), Some("FF:EU0009658145:2026-12"));
    assert_eq!(code(future).as_deref(), Some("FF:EU0009658145:2026-12"));
    // The future stated with its last trading day keys by its month alone.
    let same = walk(
        &format!("22=4|48={future}|461=FFICSX|541=20261218|711=1|309=EU0009658145|305=4"),
        5,
    );
    assert_eq!(same[0].get_instcode(), Some("FF:EU0009658145:2026-12"));
    // A calendar spread over the future and its March 2027 sibling, stated
    // in either leg order, is one strategy of two legs in code order.
    let march = "DE000F000015";
    walk(
        &format!("22=4|48={march}|461=FFICSX|200=202703|711=1|309=EU0009658145|305=4"),
        6,
    );
    // A multileg order (`AB`) is the message the dictionary structures the
    // `NoLegs(555)` group on.
    let spread = "KE:FF:EU0009658145:2026-12+FF:EU0009658145:2027-03";
    for (seq, legs) in [(7, (future, march)), (8, (march, future))] {
        let parsed: Vec<FixMsg> = codec
            .parse_lines([format!(
                "8=FIX.4.4|35=AB|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|55=FESX-SPREAD|461=KEXXXX|555=2|600=FESX|602={}|603=4|623=1|600=FESX|602={}|603=4|623=1|10=0|",
                legs.0, legs.1
            )])
            .collect::<yggdryl::Result<_>>()
            .expect("a multileg order");
        let walked: Vec<FixMsg> = codec
            .lifecycle(parsed)
            .collect::<yggdryl::Result<_>>()
            .expect("a walk");
        assert_eq!(
            walked[0].get_instcode(),
            Some(spread),
            "legs stated {legs:?}"
        );
    }
    let held = instruments.lock().expect("the registry");
    assert_eq!(
        held.len(),
        6,
        "two underlyings, the option, two futures, one spread"
    );
    let strategy = held.get(spread).expect("the spread");
    assert_eq!(
        strategy
            .legs()
            .iter()
            .map(|leg| (leg.code(), leg.ratio()))
            .collect::<Vec<_>>(),
        [
            ("FF:EU0009658145:2026-12", 1),
            ("FF:EU0009658145:2027-03", 1)
        ]
    );
    assert_eq!(
        strategy.minted_isin(),
        Some(yggdryl_market::Instrument::minted_number(spread).as_str())
    );
    // A derivative whose underlying no instrument keys is a placeholder
    // under its real ISIN, keyed again once the body can be spelled.
    drop(held);
    let orphan = "DE000C000018";
    walk(
        &format!(
            "22=4|48={orphan}|461=OCASPS|201=1|202=50|541=20261218|711=1|309=US5949181045|305=4"
        ),
        9,
    );
    {
        let held = instruments.lock().expect("the registry");
        let placeholder = held.get(orphan).expect("the placeholder");
        assert_eq!(placeholder.get_crosscode(), orphan);
        assert!(placeholder.is_placeholder());
        assert_eq!(placeholder.underlying(), None);
    }
}

/// A pipeline's lake: a table laid out as `python/tests/medallion.py` lays
/// one out - the row as Apache Doris reads an Iceberg table (decision 20:
/// an instant at microseconds under its zone, the write truncating a
/// nanosecond), numbered by this table alone, partitioned by the quarter
/// hour, sorted by the instant, the place and the hash, the identity
/// columns required - and its rows read back inside a stage's window under
/// the row the stage wrote.
#[cfg(feature = "iceberg")]
mod lake {
    use yggdryl::expression::{Ordering, Projection};
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{ArrowCastOptions, Field, IOMedia, Scheme, Serie};

    /// A stage's table numbers its own schema: the identifiers a source
    /// table's row carried come off at every depth, through the document.
    fn unnumbered(row: &Field) -> Field {
        fn strip(node: serde_json::Value) -> serde_json::Value {
            match node {
                serde_json::Value::Object(held) => serde_json::Value::Object(
                    held.into_iter()
                        .filter(|(key, _)| key != "PARQUET:field_id")
                        .map(|(key, value)| (key, strip(value)))
                        .collect(),
                ),
                serde_json::Value::Array(held) => {
                    serde_json::Value::Array(held.into_iter().map(strip).collect())
                }
                other => other,
            }
        }
        let document: serde_json::Value =
            serde_json::from_str(&row.clone().into_json().unwrap()).unwrap();
        Field::from_json(&strip(document).to_string()).unwrap()
    }

    pub(super) fn table(
        name: &str,
        row: &Field,
    ) -> (IcebergTable<LocalFolder>, std::path::PathBuf) {
        let mut schema = unnumbered(row)
            .into_scheme_compat(&Scheme::DORIS)
            .expect("the row as Doris reads it")
            .with_partition_by(["time_bucket('15 minutes', transunix) as partunix"
                .parse::<Projection>()
                .unwrap()])
            .expect("partitioned by the quarter hour");
        for name in [
            "transunix",
            "hashcode",
            "seqnum",
            "crosscode",
            "crosshashcode",
        ] {
            let mut column = schema.get_field(name).unwrap().clone();
            column.set_nullable(false);
            schema.set_field(name, column).unwrap();
        }
        schema
            .as_sort_mut()
            .set_by(
                ["partunix", "transunix", "seqnum", "hashcode"]
                    .map(|key| key.parse::<Ordering>().unwrap()),
            )
            .unwrap();
        assign_field_ids(&mut schema, 1).unwrap();
        let path = LocalFolder::temporary()
            .unwrap()
            .path()
            .unwrap()
            .join(format!("yggdryl-fix-lake-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let spec = PartitionSpec::from_schema(0, &schema).unwrap();
        (
            IcebergTable::create(
                LocalFolder::new(&path).unwrap(),
                FormatVersion::V3,
                schema.clone(),
                spec,
            )
            .unwrap(),
            path,
        )
    }

    /// The capture's day, read in the table's own order with the partition
    /// column it computed taken off and cast onto `row`, the row before the
    /// table laid it out for Doris, so a microsecond instant reads back
    /// under the row's own unit - the medallion's `stored_rows`.
    pub(super) fn rows(table: &IcebergTable<LocalFolder>, row: &Field) -> Serie {
        let options: RecordOptions = table
            .record_options()
            .unwrap()
            .with_select("* exclude (partunix)")
            .unwrap()
            .with_filter(
                "transunix >= '2026-08-14T00:00:00Z' and transunix < '2026-08-15T00:00:00Z'",
            )
            .unwrap();
        table
            .read_serie(Some(&options))
            .unwrap()
            .cast(row, ArrowCastOptions::default())
            .expect("the stored rows under the row the stage wrote")
    }
}

/// A fill a bridge logged - its row header bracketing the session, the
/// context and the sequence the lifecycle keys a session event by - parsed
/// into its order's report and its execution, stored in an Iceberg table
/// laid out as a pipeline lays one out (the row as Iceberg states it,
/// partitioned by the quarter hour, sorted by the instant, the place and
/// the hash), read back inside a window and walked, keeps the execution:
/// the three rows that left the parse are the three the walk answers,
/// whichever path they took.
#[cfg(feature = "iceberg")]
#[test]
fn an_execution_split_off_a_fill_survives_a_lake_round_trip_into_the_lifecycle() {
    crate::install::installed();
    use yggdryl::IOMedia;
    use yggdryl::graph::Element;
    use yggdryl_market::MarketDataKind;

    let registry = super::committed_registry();
    let log = b"2026-08-14 09:00:15.000 [15254-e7254b11:9f03166699:0001] [ULBridge] (INFO) Sending >> 8=FIX.4.4|9=0|35=D|49=OMS|56=VENUE|34=1|11=ORD-1|55=SYM|48=US0378331005|22=4|54=2|38=500|40=2|44=95.00|59=0|52=20260814-09:00:15.000|60=20260814-09:00:15.000|10=000|
2026-08-14 09:01:15.000 [15254-e7254b11:9f03166699:0002] [ULBridge] (INFO) Sending >> 8=FIX.4.4|9=0|35=8|49=VENUE|56=OMS|34=2|11=ORD-1|17=EXEC-1|37=ORD-1|150=F|39=2|55=SYM|48=US0378331005|22=4|54=2|38=100|32=100|31=95.08|151=0|14=100|44=95.08|52=20260814-09:01:15.000|60=20260814-09:01:15.000|10=000|
";
    let source = Buffer::from_bytes(log.to_vec())
        .with_media_type(Url::from_str("file:///fill.log").unwrap().media_type());
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl_fix::ULBRIDGE_ROWHEADER)
        .unwrap()
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    let codec = super::fixed_codec(Arc::clone(&registry))
        .with_separator(b'|')
        .with_capture_names(options.capture_names());
    let kinds = |messages: &[FixMsg]| {
        messages
            .iter()
            .map(FixMsg::marketdatakind)
            .collect::<Vec<_>>()
    };
    let expected = [
        MarketDataKind::Order,
        MarketDataKind::Order,
        MarketDataKind::Execution,
    ];
    // The text rows the lines make, as a pipeline stores them, and the FIX
    // rows the parse over them makes - the capture columns carried in front
    // of the fixed ones.
    let text_options = yggdryl::media::RecordOptions::Text(Box::new(options.clone()));
    let lake = lake::table;
    // The lines stored as the pipeline stores them, then read back.
    let lines = source.read_serie(Some(&text_options)).unwrap();
    let logs_row = lines.require_field().unwrap().clone();
    let (mut logs, logs_path) = lake("logs", &logs_row);
    let result = logs.overwrite_serie(lines, None).unwrap();
    assert_eq!(result.written_rows, 2, "{result:?}");
    let parse = || {
        codec
            .parse_text_serie(lake::rows(&logs, &logs_row))
            .unwrap()
    };
    let parsed: Vec<FixMsg> = codec
        .messages_serie(parse())
        .unwrap()
        .collect::<yggdryl::Result<_>>()
        .expect("two readable lines");
    assert_eq!(kinds(&parsed), expected, "the parse splits the fill");
    assert!(
        parsed
            .iter()
            .all(|held| held.capture().msgsesseventid().is_some()),
        "the header keys every message's session event"
    );
    let walked: Vec<FixMsg> = codec
        .lifecycle(parsed.iter().cloned().map(Ok))
        .collect::<yggdryl::Result<_>>()
        .expect("the walk over the parse");
    assert_eq!(
        kinds(&walked),
        expected,
        "the walk in memory keeps the fill"
    );

    let fix_row = parse().field().clone();
    let (mut table, path) = lake("fix", &fix_row);
    let result = table.overwrite_serie(parse().into(), None).unwrap();
    assert_eq!(result.written_rows, 3, "{result:?}");

    let stored: Vec<FixMsg> = codec
        .messages_serie(lake::rows(&table, &fix_row))
        .unwrap()
        .collect::<yggdryl::Result<_>>()
        .expect("every stored row rebuilds");
    assert_eq!(kinds(&stored), expected, "the table holds the fill");
    for (held, back) in parsed.iter().zip(&stored) {
        assert_eq!(
            held.get_uuid(),
            back.get_uuid(),
            "{}: the identity crosses",
            held.marketdatakind()
        );
        // The table holds the digests and the place as `decimal(20, 0)`,
        // and each reads back as the `uint64` it was: a zero here is what
        // folded the execution into its report's delivery.
        assert_eq!(
            held.get_hashcode(),
            back.get_hashcode(),
            "{}: the content digest crosses",
            held.marketdatakind()
        );
        assert_ne!(back.get_hashcode(), 0);
        assert_eq!(held.get_crosshashcode(), back.get_crosshashcode());
        assert_eq!(
            held.get_seqnum(),
            back.get_seqnum(),
            "{}: the place crosses",
            held.marketdatakind()
        );
        assert_eq!(held.get_crosscode(), back.get_crosscode());
        assert_eq!(
            held.capture().msgsesseventid(),
            back.capture().msgsesseventid(),
            "{}: the session event crosses",
            held.marketdatakind()
        );
    }
    let walked: Vec<FixMsg> = codec
        .lifecycle(stored.iter().cloned().map(Ok))
        .collect::<yggdryl::Result<_>>()
        .expect("the walk over the stored rows");
    assert_eq!(
        kinds(&walked),
        expected,
        "the walk over the stored rows keeps the fill"
    );
    let walked: Vec<FixMsg> = codec
        .messages_serie(codec.lifecycle_serie(lake::rows(&table, &fix_row)).unwrap())
        .unwrap()
        .collect::<yggdryl::Result<_>>()
        .expect("the serie walk over the stored rows");
    assert_eq!(
        kinds(&walked),
        expected,
        "the serie walk over the stored rows keeps the fill"
    );
    let _ = std::fs::remove_dir_all(&path);
    let _ = std::fs::remove_dir_all(&logs_path);
}

/// The bridge's whole capture, stored as a pipeline stores it and read back
/// inside its day, walks to the identities it walks to in memory. A table
/// keeps a column's name, its datatype and its `doc` and no `FIX:` key, so
/// every row comes back under columns the dictionary explains by name
/// alone - and a row read back digests as the parse did, which is what the
/// window folds a twin by: a hop logged again under the identity it
/// restates folds in the lake exactly as it folds in memory, and the
/// stored rows are the parsed messages, identity for identity.
#[cfg(feature = "iceberg")]
#[test]
fn the_capture_read_back_from_a_lake_walks_to_the_identities_it_walks_to_in_memory() {
    crate::install::installed();
    use std::collections::BTreeSet;

    use yggdryl::graph::Element;
    use yggdryl::{IOMedia, Uuid};

    let registry = super::committed_registry();
    let source = Buffer::from_bytes(include_bytes!("../../../tests/support/ulbridge.log").to_vec())
        .with_media_type(
            Url::from_str("file:///ulbridge.log")
                .expect("a URL")
                .media_type(),
        );
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl_fix::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    let codec = super::fixed_codec(Arc::clone(&registry))
        .with_exclude_msgtypes::<[&str; 0], &str>([])
        .with_capture_names(options.capture_names());
    let text_options = yggdryl::media::RecordOptions::Text(Box::new(options.clone()));
    let parse = || {
        codec
            .parse_text_serie(source.read_serie(Some(&text_options)).unwrap())
            .unwrap()
    };
    let messages = |reader: yggdryl::StreamChunkedSerie| -> Vec<FixMsg> {
        codec
            .messages_serie(reader)
            .unwrap()
            .collect::<yggdryl::Result<_>>()
            .expect("every row rebuilds")
    };
    let identities = |held: &[FixMsg]| -> BTreeSet<(i64, u64, Uuid, u64)> {
        held.iter()
            .map(|held| {
                (
                    held.get_transunix(),
                    held.get_seqnum(),
                    held.get_uuid(),
                    held.get_hashcode(),
                )
            })
            .collect()
    };
    let walk = |held: &[FixMsg], window_ms: i64| -> Vec<FixMsg> {
        codec
            .clone()
            .with_dedup_window_ms(window_ms)
            .lifecycle(held.iter().cloned().map(Ok))
            .collect::<yggdryl::Result<_>>()
            .expect("the walk")
    };
    let twins = |held: &[FixMsg]| -> BTreeSet<Uuid> {
        let mut seen = BTreeSet::new();
        held.iter()
            .filter(|held| !seen.insert(held.get_uuid()))
            .map(Element::get_uuid)
            .collect()
    };

    // In memory: the parse, and the walk over it in the order the table
    // hands it back - its instant, its place, its code - with the window
    // and without: a twin restating the identity it repeats. Two
    // messages of one instant keep the order they arrive in, so the walk
    // in memory takes the table's order to answer what the table's walk
    // answers.
    let mut parsed = messages(parse());
    assert_eq!(parsed.len(), 94 + 57, "the corpus");
    parsed.sort_by_key(|held| (held.get_transunix(), held.get_seqnum(), held.get_hashcode()));
    let walked = walk(&parsed, FixCodec::DEFAULT_DEDUP_WINDOW_MS);
    let every = walk(&parsed, 0);
    // One twin, not three, and 41 walked, not 39, since decision 21: an
    // official clock half a second or more off `SendingTime(52)` no longer
    // dates its message, so two of the three hops the old one-second delay
    // dated onto their `TransactTime(60)` now stand at their own sending
    // clocks, under identities of their own, and no window folds them -
    // under a 1,001 ms delay, the old inclusive second, the walk is 39 over
    // three twins again; the medallion's `silver.fix_messages` moved
    // 39 -> 41 the same way.
    assert_eq!(
        (walked.len(), every.len()),
        (41, 42),
        "the twin folds in memory"
    );
    assert_eq!(twins(&every).len(), 1);
    assert_eq!(twins(&walked).len(), 0);

    // Through the lake: the same rows, the same messages, the same walk.
    let row = parse().field().clone();
    let (mut table, path) = lake::table("capture", &row);
    let result = table.overwrite_serie(parse().into(), None).unwrap();
    assert_eq!(result.written_rows, 94 + 57, "{result:?}");
    let stored = messages(
        lake::rows(&table, &row)
            .into_chunked_stream(None, None)
            .unwrap(),
    );
    assert_eq!(
        identities(&stored),
        identities(&parsed),
        "the rows cross identity for identity"
    );
    // One row, settled again as the walk settles what it follows or
    // restates: the wire re-emits every field under its tag - the row keeps
    // no arrival order, so the fields are compared as a set - and the
    // identity it settles to is the one the parse stamped.
    let held = &parsed[0];
    let mut back = stored
        .iter()
        .find(|back| back.get_uuid() == held.get_uuid())
        .expect("the first parsed message is stored")
        .clone();
    let fields = |held: &FixMsg| {
        let mut fields = held.into_bytes(b'|');
        fields.pop();
        let mut fields = fields
            .split(|byte| *byte == b'|')
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>();
        fields.sort();
        fields
    };
    assert_eq!(
        fields(&back),
        fields(held),
        "a row re-emits every field the parse did"
    );
    // What each feeds its identity: every entry, pre-order, as the digest
    // reads it - the difference names what the round trip moved.
    fn feed(entries: &[yggdryl_fix::FixEntry], depth: usize, into: &mut Vec<String>) {
        for entry in entries {
            into.push(format!(
                "{}{}:{}={:?}({})",
                " ".repeat(depth),
                entry.tag(),
                entry.name(),
                entry.value(),
                entry.entries().len()
            ));
            feed(entry.entries(), depth + 1, into);
        }
    }
    let mut settled = held.clone();
    settled.finalize();
    back.finalize();
    let (mut parsed_feed, mut back_feed) = (Vec::new(), Vec::new());
    feed(settled.entries(), 0, &mut parsed_feed);
    feed(back.entries(), 0, &mut back_feed);
    parsed_feed.sort();
    back_feed.sort();
    let only_parsed: Vec<&String> = parsed_feed
        .iter()
        .filter(|line| !back_feed.contains(line))
        .collect();
    let only_back: Vec<&String> = back_feed
        .iter()
        .filter(|line| !parsed_feed.contains(line))
        .collect();
    assert_eq!(
        back.get_hashcode(),
        settled.get_hashcode(),
        "a row settles to the code the parse settles to; only the parse feeds {only_parsed:#?}, only the row {only_back:#?}"
    );
    assert_eq!(settled.get_hashcode(), held.get_hashcode());
    let lake_walked = walk(&stored, FixCodec::DEFAULT_DEDUP_WINDOW_MS);
    let lake_every = walk(&stored, 0);
    assert_eq!(
        (lake_walked.len(), lake_every.len()),
        (walked.len(), every.len()),
        "the twins fold in the lake as they fold in memory"
    );
    assert_eq!(twins(&lake_every), twins(&every), "the same twins");
    let (lake_identities, memory_identities) = (identities(&lake_walked), identities(&walked));
    let only_lake: Vec<_> = lake_identities.difference(&memory_identities).collect();
    let only_memory: Vec<_> = memory_identities.difference(&lake_identities).collect();
    assert!(
        only_lake.is_empty() && only_memory.is_empty(),
        "the walk over the stored rows answers the identities the walk in memory answers; only the lake {only_lake:#?}, only memory {only_memory:#?}"
    );
    let _ = std::fs::remove_dir_all(&path);
}

/// The lifecycle counts an order's fills once by `ExecID(17)` over the
/// chain's first stated total (decision 26): a fill redelivered at a later
/// instant counts nothing, a status reply repeating its quantity counts
/// nothing, a trade cancel takes the fill it names back, a fill that leaves
/// nothing ends the chain, and a copy of it after the chain ended starts
/// no chain from a count; the next order under the same `ClOrdID(11)`
/// starts afresh.
#[test]
fn the_lifecycle_counts_each_fill_once_and_takes_a_bust_back() {
    crate::install::installed();
    use yggdryl::graph::Element;
    use yggdryl_market::MarketDataKind;
    let codec = super::fixed_codec(super::committed_registry());
    let dec = |text: &str| -> yggdryl::Decimal { text.parse().unwrap() };
    let lines: [&[u8]; 9] = [
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30.000|11=A1|55=AAPL|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=1|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|38=100|14=0|151=100|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=2|52=20260102-10:15:31.000|11=A1|37=O1|17=E1|150=F|39=1|38=100|32=40|31=10|14=40|151=60|54=1|55=AAPL|10=0|",
        // The fill delivered again at a later instant: a hop.
        b"8=FIX.4.4|35=8|49=T|56=S|34=3|52=20260102-10:15:31.200|11=A1|37=O1|17=E1|150=F|39=1|38=100|32=40|31=10|14=40|151=60|54=1|55=AAPL|10=0|",
        // A status reply repeating the last quantity.
        b"8=FIX.4.4|35=8|49=T|56=S|34=4|52=20260102-10:15:32.000|11=A1|37=O1|17=0|150=I|39=1|38=100|32=40|31=10|14=40|151=60|54=1|55=AAPL|10=0|",
        // A bust of the fill.
        b"8=FIX.4.4|35=8|49=T|56=S|34=5|52=20260102-10:15:33.000|11=A1|37=O1|17=E3|19=E1|150=H|39=1|38=100|32=40|31=10|14=0|151=100|54=1|55=AAPL|10=0|",
        // A fill of the whole order.
        b"8=FIX.4.4|35=8|49=T|56=S|34=6|52=20260102-10:15:34.000|11=A1|37=O1|17=E4|150=F|39=2|38=100|32=100|31=10|14=100|151=0|54=1|55=AAPL|10=0|",
        // Its copy five seconds later, stating no status and no total.
        b"8=FIX.4.4|35=8|49=T|56=S|34=7|52=20260102-10:15:39.000|11=A1|37=O1|17=E4|150=F|38=100|32=100|31=10|151=0|54=1|55=AAPL|10=0|",
        // The next order under the same identifier.
        b"8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:16:00.000|11=A1|55=AAPL|54=1|38=50|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("the messages");
    let orders: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the walk")
        .into_iter()
        .filter(|held| held.marketdatakind() == MarketDataKind::Order)
        .collect();
    let read = |held: &FixMsg| {
        (
            held.get_cumqty(),
            held.get_leavesqty(),
            held.get_quantity(),
            *held.get_state(),
        )
    };
    let [order, ack, fill, again, status, bust, filled, copy, next] = orders.as_slice() else {
        panic!("nine order messages, not {}", orders.len())
    };
    assert_eq!(ack.get_prevuuid(), Some(order.get_uuid()));
    assert_eq!(
        read(fill),
        (
            Some(dec("40")),
            Some(dec("60")),
            Some(dec("60")),
            State::PartiallyFilled
        )
    );
    assert_eq!(read(again), read(fill), "counted nothing");
    assert_eq!(again.get_prevuuid(), Some(fill.get_uuid()));
    assert_eq!(read(status), read(fill), "a status reply counts nothing");
    assert_eq!(
        (bust.get_cumqty(), bust.get_leavesqty()),
        (Some(dec("0")), Some(dec("100"))),
        "the fill taken back"
    );
    assert!(bust.get_state().is_live());
    assert_eq!(
        read(filled),
        (
            Some(dec("100")),
            Some(dec("0")),
            Some(dec("0")),
            State::Filled
        )
    );
    assert_eq!(filled.get_prevuuid(), Some(bust.get_uuid()));
    assert_eq!(copy.get_prevuuid(), None, "a copy of an ended chain's fill");
    assert_eq!(*copy.get_state(), State::Filled);
    assert_eq!(next.get_prevuuid(), None, "afresh");
    assert_eq!(next.get_ordqty(), Some(dec("50")));
}

/// A trade correct moves the count by the difference it states for the fill
/// `ExecRefID(19)` names, a trade cancel naming a fill the chain never
/// counted moves nothing, a leg's report of a multi-leg order
/// (`MultiLegReportingType(442)` `2`) counts nothing whatever `LastQty(32)`
/// it states, and the multi-leg report (`442=3`) bringing the count to the
/// order quantity reads `FILLED` (decision 26).
#[test]
fn the_lifecycle_takes_a_correction_and_counts_no_leg_report() {
    crate::install::installed();
    use yggdryl_market::MarketDataKind;
    let codec = super::fixed_codec(super::committed_registry());
    let dec = |text: &str| -> yggdryl::Decimal { text.parse().unwrap() };
    let lines: [&[u8]; 8] = [
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30.000|11=A1|55=AAPL|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=1|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|38=100|14=0|151=100|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=2|52=20260102-10:15:31.000|11=A1|37=O1|17=E1|150=F|39=1|38=100|32=40|31=10|14=40|151=60|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=3|52=20260102-10:15:32.000|11=A1|37=O1|17=E2|150=F|39=1|38=100|32=30|31=10|14=70|151=30|54=1|55=AAPL|10=0|",
        // The first fill corrected from 40 to 50.
        b"8=FIX.4.4|35=8|49=T|56=S|34=4|52=20260102-10:15:33.000|11=A1|37=O1|17=E5|19=E1|150=G|39=1|38=100|32=50|31=10|14=80|151=20|54=1|55=AAPL|10=0|",
        // A bust of a fill the chain never counted.
        b"8=FIX.4.4|35=8|49=T|56=S|34=5|52=20260102-10:15:34.000|11=A1|37=O1|17=E8|19=EX|150=H|39=1|38=100|32=10|31=10|14=80|151=20|54=1|55=AAPL|10=0|",
        // A leg's report: no fill of the order.
        b"8=FIX.4.4|35=8|49=T|56=S|34=6|52=20260102-10:15:35.000|11=A1|37=O1|17=E6|150=F|39=1|442=2|38=100|32=20|31=10|14=80|151=20|54=1|55=AAPL|10=0|",
        // The multi-leg report of the last twenty.
        b"8=FIX.4.4|35=8|49=T|56=S|34=7|52=20260102-10:15:36.000|11=A1|37=O1|17=E7|150=F|39=1|442=3|38=100|32=20|31=10|14=100|151=0|54=1|55=AAPL|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("the messages");
    let orders: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the walk")
        .into_iter()
        .filter(|held| held.marketdatakind() == MarketDataKind::Order)
        .collect();
    let read = |held: &FixMsg| (held.get_cumqty(), held.get_leavesqty(), *held.get_state());
    let [_, _, _, second, corrected, bust, leg, multileg] = orders.as_slice() else {
        panic!("eight order messages, not {}", orders.len())
    };
    assert_eq!(
        read(second),
        (Some(dec("70")), Some(dec("30")), State::PartiallyFilled)
    );
    assert_eq!(
        read(corrected),
        (Some(dec("80")), Some(dec("20")), State::PartiallyFilled),
        "the first fill counted at 50"
    );
    assert_eq!(read(bust), read(corrected), "a bust naming nothing counted");
    assert_eq!(read(leg), read(corrected), "a leg's report counts nothing");
    assert_eq!(
        read(multileg),
        (Some(dec("100")), Some(dec("0")), State::Filled)
    );
}

/// The reports of two orders on the two sides of one match, each stating
/// the match's one `ExecID(17)`: two executions, the sell's no copy of the
/// buy's ended one.
#[test]
fn the_two_sides_of_one_match_under_one_execid_are_two_executions() {
    crate::install::installed();
    use yggdryl::graph::Element;
    use yggdryl_market::MarketDataKind;
    let codec = super::fixed_codec(super::committed_registry());
    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=8|49=T|56=S|34=1|52=20260102-10:15:31.000|11=B1|37=OB|17=X1|150=F|39=2|38=40|32=40|31=10|14=40|151=0|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=2|52=20260102-10:15:31.000|11=S1|37=OS|17=X1|150=F|39=2|38=40|32=40|31=10|14=40|151=0|54=2|55=AAPL|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("the messages");
    let executions: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the walk")
        .into_iter()
        .filter(|held| held.marketdatakind() == MarketDataKind::Execution)
        .collect();
    let [buy, sell] = executions.as_slice() else {
        panic!("two executions, not {}", executions.len())
    };
    assert_ne!(buy.get_uuid(), sell.get_uuid());
    assert_ne!(buy.get_crosscode(), sell.get_crosscode());
}

/// Order `00079132557GLXC0`'s four frames as the capture states them: the
/// first stated total anchors the count (319 traded before), the exchange-
/// side frames of fills 467 and 468 state the child's `LeavesQty(151)` of
/// nothing and no status, which the count contradicts - the order ends
/// `PARTIALLY_FILLED` at 472 of 600, 128 left - and the client-side frame
/// of fill 467 counts nothing (decisions 26 and 29).
#[test]
fn the_lifecycle_reads_the_captures_order_557_as_partially_filled_by_its_count() {
    crate::install::installed();
    use yggdryl::graph::Element;
    use yggdryl_market::MarketDataKind;
    let codec = super::fixed_codec(super::committed_registry());
    let dec = |text: &str| -> yggdryl::Decimal { text.parse().unwrap() };
    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=8|49=X|56=Y|34=1|52=20260814-12:46:39.761|38=600|14=340|39=1|151=260|150=F|11=00079132557GLXC0.9|17=00064703457GBYZ0|32=21|31=83.08|37=00079132557GLXC0|54=1|55=ABBN|60=20260814-12:46:39.743|10=0|",
        b"8=FIX.4.4|35=8|49=X|56=Y|34=2|52=20260814-12:46:39.762|38=600|151=0|150=F|17=00064703467GBYZ0|32=57|31=83.08|37=00079132557GLXC0|54=1|55=ABBN|60=20260814-12:46:39.750|10=0|",
        b"8=FIX.4.4|35=8|49=X|56=Y|34=3|52=20260814-12:46:39.763|38=600|14=397|39=1|151=203|150=F|11=00079132557GLXC0.9|17=00064703467GBYZ0|32=57|31=83.08|37=00079132557GLXC0|54=1|55=ABBN|60=20260814-12:46:39.751|10=0|",
        b"8=FIX.4.4|35=8|49=X|56=Y|34=4|52=20260814-12:46:39.771|38=600|151=0|150=F|17=00064703468GBYZ0|32=75|31=83.08|37=00079132557GLXC0|54=1|55=ABBN|60=20260814-12:46:39.752|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec
        .parse_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("the messages");
    assert_eq!(
        *parsed[2].get_state(),
        State::Filled,
        "the frame's own reading"
    );
    let orders: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the walk")
        .into_iter()
        .filter(|held| held.marketdatakind() == MarketDataKind::Order)
        .collect();
    let read = |held: &FixMsg| (held.get_cumqty(), held.get_leavesqty(), *held.get_state());
    let [first, exchange, client, last] = orders.as_slice() else {
        panic!("four reports, not {}", orders.len())
    };
    assert_eq!(
        read(first),
        (Some(dec("340")), Some(dec("260")), State::PartiallyFilled)
    );
    assert_eq!(
        read(exchange),
        (Some(dec("397")), Some(dec("203")), State::PartiallyFilled),
        "the count, not the child's remainder"
    );
    assert_eq!(
        read(client),
        (Some(dec("397")), Some(dec("203")), State::PartiallyFilled),
        "fill 467 counted once"
    );
    assert_eq!(
        read(last),
        (Some(dec("472")), Some(dec("128")), State::PartiallyFilled)
    );
    assert_eq!(last.get_prevuuid(), Some(client.get_uuid()));
}
