//! The specification's tables read as implications: each rule answered once
//! from a hand-written line, refused where the answer is not certain, and
//! settled in one pass.

use super::OneMessage;

use std::path::PathBuf;
use std::sync::Arc;

use yggdryl::holder::local::Folder;
use yggdryl::types::State;
use yggdryl::{FixCodec, FixMsg, FixRegistry, Scalar};

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

/// One line read with enrichment on, then enriched again to prove a second
/// pass changes nothing: every chain of rules reaches its end in one pass.
fn settled(reader: &FixCodec, line: &[u8]) -> FixMsg {
    let once = reader.one_line(line, true).expect("a readable line");
    let twice = reader.enrich_message(once.clone()).expect("a second pass");
    assert_eq!(
        once,
        twice,
        "a second pass over {}",
        String::from_utf8_lossy(line)
    );
    once
}

/// The text one tag holds, for the assertions that read a spelling.
fn text(message: &FixMsg, tag: i32) -> Option<&str> {
    message.get_by_tag(tag).and_then(Scalar::as_str)
}

/// The integer one tag holds.
fn integer(message: &FixMsg, tag: i32) -> Option<i128> {
    message.get_by_tag(tag).and_then(Scalar::as_i128)
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
fn an_identifier_names_the_standard_that_closes_it() {
    let reader = reader();
    // ISO 6166 closes a number with a check digit, and so do CUSIP and
    // SEDOL: a `SecurityID` one of them closes has stated its own source.
    let isin = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=US0378331005|10=0|");
    assert_eq!(text(&isin, 22), Some("4"));
    assert_eq!(
        text(&isin, yggdryl::ISINCODE_TAG_NAME.0),
        Some("US0378331005")
    );
    assert_eq!(text(&isin, 470), Some("US"), "the country the prefix names");

    let cusip = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=037833100|10=0|");
    assert_eq!(text(&cusip, 22), Some("1"));
    assert_eq!(cusip.get_by_tag(yggdryl::ISINCODE_TAG_NAME.0), None);
    assert_eq!(cusip.get_by_tag(470), None);

    let sedol = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=B0YBKJ7|10=0|");
    assert_eq!(text(&sedol, 22), Some("2"));

    // Case does not change what a check digit closes.
    let folded = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=us0378331005|10=0|");
    assert_eq!(text(&folded, 22), Some("4"));
    assert_eq!(
        text(&folded, yggdryl::ISINCODE_TAG_NAME.0),
        Some("US0378331005")
    );

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
        assert_eq!(held.get_by_tag(yggdryl::ISINCODE_TAG_NAME.0), None);
    }

    // A stated source wins over what the value would validate as, and an
    // ISIN under another source is not read as one.
    let stated = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=US0378331005|22=1|10=0|");
    assert_eq!(text(&stated, 22), Some("1"));
    assert_eq!(stated.get_by_tag(yggdryl::ISINCODE_TAG_NAME.0), None);

    // The rules read codes. A bridge row spelling the source in its own
    // word has stated one, which stands, and `isin` is not the code `4`: the
    // dictionary names that code `ISINNumber`, so nothing translated it, and
    // the ISIN rule reads the code alone.
    let worded = settled(
        &reader,
        b"MSGTYPE=D|CLORDID=A|SECURITYID=CH0012221716|SECURITYIDSOURCE=isin",
    );
    assert_eq!(text(&worded, 22), Some("isin"));
    assert_eq!(worded.get_by_tag(yggdryl::ISINCODE_TAG_NAME.0), None);
}

#[test]
fn an_isin_reaches_its_column_from_wherever_the_message_put_it() {
    let reader = reader();
    // The alternate identifier whose source says ISIN is the ISIN, and once
    // the row holds one it holds the primary identifier and its source too.
    let held = settled(&reader, &alternate("CH0012221716", "4"));
    assert_eq!(
        text(&held, yggdryl::ISINCODE_TAG_NAME.0),
        Some("CH0012221716")
    );
    assert_eq!(text(&held, 48), Some("CH0012221716"));
    assert_eq!(text(&held, 22), Some("4"));
    assert_eq!(text(&held, 470), Some("CH"));

    // A message stating an ISIN in both places states it in `SecurityID`,
    // as the column is defined: the alternate answers only where the primary
    // did not, and the country is the primary's.
    let both = settled(
        &reader,
        b"MSGTYPE=D|#CLORDID=A|#SECURITYID=US0378331005|#NOSECURITYALTID=1|#NOSECURITYALTID[0]=SECURITYALTID=CH0012221716\x04\x03SECURITYALTIDSOURCE=4",
    );
    assert_eq!(text(&both, 22), Some("4"));
    assert_eq!(
        text(&both, yggdryl::ISINCODE_TAG_NAME.0),
        Some("US0378331005")
    );
    assert_eq!(text(&both, 470), Some("US"));
    // Under another source the primary is no ISIN, and the alternate is.
    let sourced = settled(
        &reader,
        b"MSGTYPE=D|#CLORDID=A|#SECURITYID=037833100|#NOSECURITYALTID=1|#NOSECURITYALTID[0]=SECURITYALTID=CH0012221716\x04\x03SECURITYALTIDSOURCE=4",
    );
    assert_eq!(text(&sourced, 22), Some("1"));
    assert_eq!(text(&sourced, 48), Some("037833100"));
    assert_eq!(
        text(&sourced, yggdryl::ISINCODE_TAG_NAME.0),
        Some("CH0012221716")
    );
    assert_eq!(text(&sourced, 470), Some("CH"));

    // A bridge row stating only the crate's own column has stated the
    // primary identifier.
    let bridge = settled(&reader, b"MSGTYPE=D|CLORDID=A|ISINCODE=GB0002634946");
    assert_eq!(text(&bridge, 48), Some("GB0002634946"));
    assert_eq!(text(&bridge, 22), Some("4"));
    assert_eq!(text(&bridge, 470), Some("GB"));

    // An international prefix is an agency and not a country.
    for id in ["XS0000000009", "EU0000000008"] {
        let held = settled(&reader, &alternate(id, "4"));
        assert_eq!(text(&held, yggdryl::ISINCODE_TAG_NAME.0), Some(id));
        assert_eq!(held.get_by_tag(470), None, "{id}");
    }

    // An alternate identifier under another source is not an ISIN, and one
    // the check digit does not close is nothing at all.
    let cusip = settled(&reader, &alternate("037833100", "1"));
    assert_eq!(cusip.get_by_tag(yggdryl::ISINCODE_TAG_NAME.0), None);
    assert_eq!(cusip.get_by_tag(48), None);
    let masked = settled(&reader, &alternate("XX0000000001", "4"));
    assert_eq!(masked.get_by_tag(yggdryl::ISINCODE_TAG_NAME.0), None);
    assert_eq!(masked.get_by_tag(48), None);
    assert_eq!(masked.get_by_tag(22), None);
}

#[test]
fn a_symbol_is_what_an_exchange_or_bloomberg_called_the_instrument() {
    let reader = reader();
    let exchange = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=ABBN|22=8|10=0|");
    assert_eq!(text(&exchange, 55), Some("ABBN"));
    let bloomberg = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=ABBN SW|22=A|10=0|");
    assert_eq!(text(&bloomberg, 55), Some("ABBN SW"));

    // Under an ISIN the primary identifier is no symbol, but an exchange's
    // alternate identifier is.
    let alternate = settled(
        &reader,
        b"MSGTYPE=D|#CLORDID=A|#SECURITYID=CH0012221716|#SECURITYIDSOURCE=4|#NOSECURITYALTID=1|#NOSECURITYALTID[0]=SECURITYALTID=ABBN\x04\x03SECURITYALTIDSOURCE=8",
    );
    assert_eq!(text(&alternate, 55), Some("ABBN"));
    let isin = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=CH0012221716|22=4|10=0|");
    assert_eq!(isin.get_by_tag(55), None);

    let stated = settled(&reader, b"8=FIX.4.4|35=D|11=A|55=NOVN|48=ABBN|22=8|10=0|");
    assert_eq!(text(&stated, 55), Some("NOVN"));
}

#[test]
fn a_cfi_and_a_security_type_state_each_other() {
    let reader = reader();
    // Appendix 6-D at its category level: the equity category is common
    // stock, and the dictionary files common stock under the equity product.
    let share = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=ESVTFR|10=0|");
    assert_eq!(text(&share, 167), Some("CS"));
    assert_eq!(integer(&share, 460), Some(5));
    let folded = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=esvtfr|10=0|");
    assert_eq!(text(&folded, 167), Some("CS"));

    // An option's second character is its exercise, and an option on a
    // future is its own security type; neither is a product of its own.
    let call = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=OCFXXX|10=0|");
    assert_eq!(text(&call, 167), Some("OOF"));
    assert_eq!(integer(&call, 201), Some(1));
    assert_eq!(call.get_by_tag(460), None);
    let put = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=OPEICS|10=0|");
    assert_eq!(text(&put, 167), Some("OPT"));
    assert_eq!(integer(&put, 201), Some(0));
    let open = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=HXXXXX|10=0|");
    assert_eq!(text(&open, 167), Some("OPT"));
    assert_eq!(
        open.get_by_tag(201),
        None,
        "an exercise the code leaves open"
    );

    // A financing's category is one product, and its group is one type.
    let repo = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=LRXXXX|10=0|");
    assert_eq!(text(&repo, 167), Some("REPO"));
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
        (b"8=FIX.4.4|35=D|11=A|167=FXSPOT|10=0|", "IFXXXX", Some(4)),
    ] {
        let held = settled(&reader, line);
        let spelled = String::from_utf8_lossy(line);
        assert_eq!(text(&held, 461), Some(cfi), "{spelled}");
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
    assert_eq!(text(&stated, 167), Some("PS"));
    assert_eq!(integer(&stated, 460), Some(12));
}

#[test]
fn the_crates_market_and_state_columns_are_stated_on_the_message() {
    let reader = reader();
    for (line, market) in [
        (
            &b"8=FIX.4.4|35=D|11=A|207=XSWX|100=XNAS|30=XLON|10=0|"[..],
            "XSWX",
        ),
        (b"8=FIX.4.4|35=D|11=A|100=XNAS|30=XLON|10=0|", "XNAS"),
        (b"8=FIX.4.4|35=D|11=A|30=XLON|10=0|", "XLON"),
    ] {
        let held = settled(&reader, line);
        assert_eq!(text(&held, yggdryl::MICCODE_TAG_NAME.0), Some(market));
    }
    let silent = settled(&reader, b"8=FIX.4.4|35=D|11=A|10=0|");
    assert_eq!(silent.get_by_tag(yggdryl::MICCODE_TAG_NAME.0), None);

    // The column spells a state by its rank, whichever code stated it.
    let status = settled(&reader, b"8=FIX.4.4|35=8|39=1|150=F|10=0|");
    assert_eq!(
        text(&status, yggdryl::STATE_TAG_NAME.0),
        Some(state("1").as_str())
    );
    let trade = settled(&reader, b"8=FIX.4.4|35=8|150=F|10=0|");
    assert_eq!(
        text(&trade, yggdryl::STATE_TAG_NAME.0),
        Some(state("F").as_str())
    );
    assert_eq!(trade.get_by_tag(39), None, "a trade alone says no status");
}

#[test]
fn a_currency_states_the_one_a_trade_settles_in_and_back() {
    let reader = reader();
    let settling = settled(&reader, b"8=FIX.4.4|35=8|120=USD|10=0|");
    assert_eq!(text(&settling, 15), Some("USD"));
    let dealt = settled(&reader, b"8=FIX.4.4|35=8|15=EUR|10=0|");
    assert_eq!(text(&dealt, 120), Some("EUR"));
    let both = settled(&reader, b"8=FIX.4.4|35=8|15=EUR|120=USD|10=0|");
    assert_eq!(text(&both, 15), Some("EUR"));
    assert_eq!(text(&both, 120), Some("USD"));
}

#[test]
fn an_order_stating_no_time_in_force_is_a_day_order() {
    let reader = reader();
    for line in [
        &b"8=FIX.4.4|35=D|11=A|10=0|"[..],
        b"8=FIX.4.4|35=G|11=B|41=A|10=0|",
        b"8=FIX.4.4|35=8|37=A|10=0|",
    ] {
        let held = settled(&reader, line);
        assert_eq!(
            text(&held, 59),
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
    assert_eq!(text(&stated, 59), Some("1"));
}

#[test]
fn a_report_states_its_status_where_its_execution_type_or_its_quantities_do() {
    let reader = reader();
    // The values the two code sets spell alike.
    let new = settled(&reader, b"8=FIX.4.4|35=8|150=0|10=0|");
    assert_eq!(text(&new, 39), Some(state("0").as_str()));
    let pending = settled(&reader, b"8=FIX.4.4|35=8|150=A|10=0|");
    assert_eq!(text(&pending, 39), Some(state("A").as_str()));
    // `D` is Restated in one and AcceptedForBidding in the other.
    let restated = settled(&reader, b"8=FIX.4.4|35=8|150=D|10=0|");
    assert_eq!(restated.get_by_tag(39), None);

    // A trade says what happened, and the quantities say where that leaves
    // the order: nothing left is filled, something left and something done
    // is partially filled.
    let filled = settled(&reader, b"8=FIX.4.4|35=8|150=F|151=0|14=100|10=0|");
    assert_eq!(text(&filled, 39), Some(state("2").as_str()));
    let corrected = settled(&reader, b"8=FIX.4.4|35=8|150=G|151=0|10=0|");
    assert_eq!(text(&corrected, 39), Some(state("2").as_str()));
    let partial = settled(&reader, b"8=FIX.4.4|35=8|150=F|151=60|14=40|10=0|");
    assert_eq!(text(&partial, 39), Some(state("1").as_str()));
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
    }

    // Only a report speaks for an order's status, and a stated one stands.
    let order = settled(&reader, b"8=FIX.4.4|35=D|11=A|150=0|10=0|");
    assert_eq!(order.get_by_tag(39), None);
    let stated = settled(&reader, b"8=FIX.4.4|35=8|39=2|150=F|151=60|14=40|10=0|");
    assert_eq!(text(&stated, 39), Some(state("2").as_str()));

    // A status read off the execution type is a status the remainder rule
    // reads: one pass answers both.
    let chained = settled(&reader, b"8=FIX.4.4|35=8|150=0|38=100|14=0|10=0|");
    assert_eq!(text(&chained, 39), Some(state("0").as_str()));
    assert_eq!(chained.by_tag(151).unwrap(), &Scalar::from(100.0_f64));
    assert_eq!(
        text(&chained, yggdryl::STATE_TAG_NAME.0),
        Some(state("0").as_str())
    );
}

#[test]
fn a_value_that_would_not_type_is_filled_in_place_and_the_wire_is_untouched() {
    let reader = reader();
    // `PutOrCall` is a number, so `abc` types as null while the entry keeps
    // the text; the option's own code then says it is a call, and the answer
    // takes the null's place rather than standing beside it.
    const LINE: &[u8] = b"8=FIX.4.4|35=D|11=A|461=OCXXXX|201=abc|10=0|";
    let bare = reader.one_line(LINE, false).expect("a readable line");
    assert_eq!(bare.get_by_tag(201), Some(&Scalar::Null));
    let held = settled(&reader, LINE);
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
    // The entries are what arrived, whether or not the row was filled.
    assert_eq!(bare.entries(), held.entries());
    assert_eq!(held.into_bytes(b'|'), LINE);
    let entry = held
        .entries()
        .iter()
        .find(|entry| entry.tag() == 201)
        .expect("the pair still arrived");
    assert_eq!(entry.value().as_str(), Some("abc"));
}
