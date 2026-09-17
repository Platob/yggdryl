//! The specification's tables read as implications: each carried by the
//! field it fills as a `fix:derivation`, answered once from a hand-written
//! line, refused where the answer is not certain, and settled in one pass.

use super::SoleMessage;

use std::path::PathBuf;
use std::sync::Arc;

use yggdryl::expression::Term;
use yggdryl::holder::Buffer;
use yggdryl::holder::local::Folder;
use yggdryl::media::text::{TextLine, TextOptions, read_text_lines};
use yggdryl::types::{Isin, State};
use yggdryl::{
    DataType, FixCategory, FixCodec, FixMsg, FixRegistry, Scalar, StringEnum, Timezone, Url,
    fix_schema,
};

fn reader() -> FixCodec {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder is a local path");
    super::fixed_codec(Arc::new(
        FixRegistry::from_handle(&folder).expect("the committed dictionary loads"),
    ))
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

/// The integer one tag holds.
fn integer(message: &FixMsg, tag: i32) -> Option<i128> {
    message.get_by_tag(tag).as_ref().and_then(Scalar::as_i128)
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
    assert_eq!(text(&isin, 22).as_deref(), Some("4"));
    assert_eq!(
        text(&isin, yggdryl::ISINCODE_TAG_NAME.0).as_deref(),
        Some("US0378331005")
    );
    assert_eq!(
        text(&isin, 470).as_deref(),
        Some("US"),
        "the country the prefix names"
    );

    let cusip = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=037833100|10=0|");
    assert_eq!(text(&cusip, 22).as_deref(), Some("1"));
    assert_eq!(cusip.get_by_tag(yggdryl::ISINCODE_TAG_NAME.0), None);
    assert_eq!(cusip.get_by_tag(470), None);

    let sedol = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=B0YBKJ7|10=0|");
    assert_eq!(text(&sedol, 22).as_deref(), Some("2"));

    // Case does not change what a check digit closes.
    let folded = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=us0378331005|10=0|");
    assert_eq!(text(&folded, 22).as_deref(), Some("4"));
    assert_eq!(
        text(&folded, yggdryl::ISINCODE_TAG_NAME.0).as_deref(),
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
    assert_eq!(text(&stated, 22).as_deref(), Some("1"));
    assert_eq!(stated.get_by_tag(yggdryl::ISINCODE_TAG_NAME.0), None);

    // The rules read codes. A bridge row spelling the source in its own
    // word has stated one, which stands, and `isin` is not the code `4`: the
    // dictionary names that code `ISINNumber`, so nothing translated it, and
    // the ISIN rule reads the code alone.
    let worded = settled(
        &reader,
        b"MSGTYPE=D|CLORDID=A|SECURITYID=CH0012221716|SECURITYIDSOURCE=isin",
    );
    assert_eq!(text(&worded, 22).as_deref(), Some("isin"));
    assert_eq!(worded.get_by_tag(yggdryl::ISINCODE_TAG_NAME.0), None);
}

#[test]
fn an_isin_reaches_its_column_from_wherever_the_message_put_it() {
    let reader = reader();
    // The alternate identifier whose source says ISIN is the ISIN, and once
    // the row holds one it holds the primary identifier and its source too.
    let held = settled(&reader, &alternate("CH0012221716", "4"));
    assert_eq!(
        text(&held, yggdryl::ISINCODE_TAG_NAME.0).as_deref(),
        Some("CH0012221716")
    );
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
    assert_eq!(
        text(&both, yggdryl::ISINCODE_TAG_NAME.0).as_deref(),
        Some("US0378331005")
    );
    assert_eq!(text(&both, 470).as_deref(), Some("US"));
    // Under another source the primary is no ISIN, and the alternate is.
    let sourced = settled(
        &reader,
        b"MSGTYPE=D|#CLORDID=A|#SECURITYID=037833100|#NOSECURITYALTID=1|#NOSECURITYALTID[0]=SECURITYALTID=CH0012221716\x04\x03SECURITYALTIDSOURCE=4",
    );
    assert_eq!(text(&sourced, 22).as_deref(), Some("1"));
    assert_eq!(text(&sourced, 48).as_deref(), Some("037833100"));
    assert_eq!(
        text(&sourced, yggdryl::ISINCODE_TAG_NAME.0).as_deref(),
        Some("CH0012221716")
    );
    assert_eq!(text(&sourced, 470).as_deref(), Some("CH"));

    // A bridge row stating only the crate's own column has stated the
    // primary identifier.
    let bridge = settled(&reader, b"MSGTYPE=D|CLORDID=A|ISINCODE=GB0002634946");
    assert_eq!(text(&bridge, 48).as_deref(), Some("GB0002634946"));
    assert_eq!(text(&bridge, 22).as_deref(), Some("4"));
    assert_eq!(text(&bridge, 470).as_deref(), Some("GB"));

    // An international prefix is an agency and not a country.
    for id in ["XS0000000009", "EU0000000008"] {
        let held = settled(&reader, &alternate(id, "4"));
        assert_eq!(
            text(&held, yggdryl::ISINCODE_TAG_NAME.0).as_deref(),
            Some(id)
        );
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
fn a_cfi_and_a_security_type_state_each_other() {
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
        (b"8=FIX.4.4|35=D|11=A|167=FXSPOT|10=0|", "IFXXXX", Some(4)),
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
        assert_eq!(
            text(&held, yggdryl::MICCODE_TAG_NAME.0).as_deref(),
            Some(market)
        );
    }
    let silent = settled(&reader, b"8=FIX.4.4|35=D|11=A|10=0|");
    assert_eq!(silent.get_by_tag(yggdryl::MICCODE_TAG_NAME.0), None);

    // The column spells a state by its rank, whichever code stated it.
    let status = settled(&reader, b"8=FIX.4.4|35=8|39=1|150=F|10=0|");
    assert_eq!(
        text(&status, yggdryl::STATE_TAG_NAME.0).as_deref(),
        Some(state("1").as_str())
    );
    let trade = settled(&reader, b"8=FIX.4.4|35=8|150=F|10=0|");
    assert_eq!(
        text(&trade, yggdryl::STATE_TAG_NAME.0).as_deref(),
        Some(state("F").as_str())
    );
    assert_eq!(trade.get_by_tag(39), None, "a trade alone says no status");
}

#[test]
fn a_currency_states_the_one_a_trade_settles_in_and_back() {
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
    // is partially filled.
    let filled = settled(&reader, b"8=FIX.4.4|35=8|150=F|151=0|14=100|10=0|");
    assert_eq!(text(&filled, 39).as_deref(), Some("2"));
    let corrected = settled(&reader, b"8=FIX.4.4|35=8|150=G|151=0|10=0|");
    assert_eq!(text(&corrected, 39).as_deref(), Some("2"));
    let partial = settled(&reader, b"8=FIX.4.4|35=8|150=F|151=60|14=40|10=0|");
    assert_eq!(text(&partial, 39).as_deref(), Some("1"));
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
    assert_eq!(text(&stated, 39).as_deref(), Some("2"));

    // A status read off the execution type is a status the remainder rule
    // reads: one pass answers both.
    let chained = settled(&reader, b"8=FIX.4.4|35=8|150=0|38=100|14=0|10=0|");
    assert_eq!(text(&chained, 39).as_deref(), Some("0"));
    assert_eq!(chained.by_tag(151).unwrap(), Scalar::from(100.0_f64));
    assert_eq!(
        text(&chained, yggdryl::STATE_TAG_NAME.0).as_deref(),
        Some(state("0").as_str())
    );
}

#[test]
fn a_value_that_would_not_type_is_filled_in_place() {
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

/// The committed dictionary, owned, for the cases that edit a field.
fn committed() -> FixRegistry {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder is a local path");
    FixRegistry::from_handle(&folder).expect("the committed dictionary loads")
}

#[test]
fn a_derivation_edited_on_a_registry_field_is_what_the_reader_fills_by() {
    let mut registry = committed();
    // The shipped rule: what is left is what was ordered minus what was
    // done. A desk whose venue reports `LeavesQty` in lots of ten edits the
    // field, and nothing else.
    let mut leaves = registry.field_by_tag(151).expect("LeavesQty").clone();
    let shipped = leaves
        .as_fix()
        .derivation()
        .expect("a readable derivation")
        .expect("a shipped derivation");
    assert!(shipped.to_string().contains("orderqty - cumqty"));
    leaves
        .as_fix_mut()
        .set_derivation(
            &"case when msgtype in ('8', '9') then (orderqty - cumqty) / 10 end"
                .parse::<Term>()
                .expect("a term"),
        )
        .expect("a derivation is stored");
    registry.update(leaves).expect("the field updates");

    let reader = super::fixed_codec(Arc::new(registry));
    let held = settled(&reader, b"8=FIX.4.4|35=8|39=0|38=100|14=20|10=0|");
    assert_eq!(held.by_tag(151).unwrap(), Scalar::from(8.0_f64));

    // Removing it silences the fill. `update` merges, and a stored key the
    // incoming field omits is kept as every `fix:` key is, so the removal
    // lands through `update_definition`, which replaces the definition
    // whole; a reader built over the registry as it stands then fills by
    // the registry as it stands then.
    let mut registry = committed();
    let mut leaves = registry.field_by_tag(151).expect("LeavesQty").clone();
    assert!(
        leaves
            .as_fix_mut()
            .remove_derivation()
            .expect("removable")
            .is_some()
    );
    registry.update(leaves.clone()).expect("the field updates");
    assert!(
        registry
            .field_by_tag(151)
            .expect("still there")
            .as_fix()
            .derivation()
            .expect("readable")
            .is_some(),
        "a merge keeps the stored derivation"
    );
    registry
        .insert(leaves)
        .expect("the definition is replaced whole");
    assert!(
        registry
            .field_by_tag(151)
            .expect("still there")
            .as_fix()
            .derivation()
            .expect("readable")
            .is_none()
    );
    let reader = super::fixed_codec(Arc::new(registry));
    let held = settled(&reader, b"8=FIX.4.4|35=8|39=0|38=100|14=20|10=0|");
    assert_eq!(held.get_by_tag(151), None, "nothing derives it now");
}

#[test]
fn a_malformed_derivation_refuses_at_insert_and_update_naming_the_field() {
    // Insert and update validate as a load does: a text that is not a term
    // is refused by the registry, naming the field, and the registry stands.
    let mut registry = committed();
    let mut gross = registry.field_by_tag(381).expect("GrossTradeAmt").clone();
    gross
        .insert_metadata("fix:derivation", "lastqty *")
        .expect("any text can be stored on a field");
    let refused = registry.update(gross).expect_err("a malformed derivation");
    let rendered = refused.to_string();
    assert!(rendered.contains("grosstradeamt"), "{rendered}");
    assert!(rendered.contains("fix:derivation"), "{rendered}");
    assert!(
        registry
            .field_by_tag(381)
            .expect("still there")
            .as_fix()
            .derivation()
            .expect("readable")
            .is_some(),
        "the stored field is untouched"
    );

    let mut fresh = DataType::Float64.nullable_field("notional");
    fresh.as_fix_mut().set_tag(9_381).expect("a tag");
    fresh
        .insert_metadata("fix:derivation", "case when")
        .expect("stored");
    let refused = registry.insert(fresh).expect_err("refused at insert");
    assert!(refused.to_string().contains("notional"), "{refused}");
    assert!(registry.get_field_by_tag(9_381).is_none());

    // The field's own reader says the same of the same text.
    let mut broken = DataType::Float64.nullable_field("notional");
    broken.as_fix_mut().set_tag(9_381).expect("a tag");
    broken
        .insert_metadata("fix:derivation", "lastqty *")
        .expect("stored");
    let refused = broken.as_fix().derivation().expect_err("not a term");
    assert!(refused.to_string().contains("fix:derivation"), "{refused}");
}

#[test]
fn a_derivation_naming_what_the_dictionary_lacks_is_refused_at_compile() {
    // A name no field or group of the registry answers to is a fact about
    // the dictionary, not about any message: the first enrichment refuses,
    // naming the field, and no message is read for it.
    let mut registry = committed();
    let mut gross = registry.field_by_tag(381).expect("GrossTradeAmt").clone();
    gross
        .as_fix_mut()
        .set_derivation(&"lastqty * nosuchfield".parse::<Term>().expect("a term"))
        .expect("a well-formed term is stored");
    registry.update(gross).expect("the text is a term");
    let reader = super::fixed_codec(Arc::new(registry));
    let refused = reader
        .sole_line(b"8=FIX.4.4|35=8|37=A|32=10|31=2|10=0|")
        .expect_err("refused at compile");
    let rendered = refused.to_string();
    assert!(rendered.contains("grosstradeamt"), "{rendered}");
    assert!(rendered.contains("nosuchfield"), "{rendered}");

    // So is one that names a real field the term cannot read that way.
    let mut registry = committed();
    let mut gross = registry.field_by_tag(381).expect("GrossTradeAmt").clone();
    gross
        .as_fix_mut()
        .set_derivation(&"lastqty * symbol".parse::<Term>().expect("a term"))
        .expect("stored");
    registry.update(gross).expect("the text is a term");
    let reader = super::fixed_codec(Arc::new(registry));
    let refused = reader
        .sole_line(b"8=FIX.4.4|35=8|37=A|32=10|31=2|10=0|")
        .expect_err("refused at compile");
    assert!(refused.to_string().contains("grosstradeamt"), "{refused}");
}

#[test]
fn an_absent_input_is_silence_and_a_stated_value_is_never_overwritten() {
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
    assert_eq!(whole.by_tag(381).unwrap(), Scalar::from(25.0_f64));

    // A stated value stands whatever the derivation would say, and a stated
    // null is not a stated value: the derivation fills it in place.
    let stated = settled(&reader, b"8=FIX.4.4|35=8|37=A|32=10|31=2.5|381=99|10=0|");
    assert_eq!(stated.by_tag(381).unwrap(), Scalar::from(99.0_f64));
    let nulled = settled(&reader, b"8=FIX.4.4|35=8|37=A|32=10|31=2.5|381=abc|10=0|");
    assert_eq!(nulled.by_tag(381).unwrap(), Scalar::from(25.0_f64));
    // The wire re-emits the message as it now stands, so the column the
    // fill landed in is what the pair says.
    assert_eq!(
        String::from_utf8(nulled.into_bytes(b'|')).unwrap(),
        "8=FIX.4.4|35=8|37=A|32=10|31=2.5|381=25|10=0|59=0|"
    );
}

#[test]
fn a_chain_resolves_in_one_pass_whatever_order_its_fields_fall_in() {
    let reader = reader();
    // `securityid` -> `securityidsource` -> `isincode` -> `countryofissue`:
    // the source is read off the identifier, the column off the source, the
    // country off the column.
    let chain = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=GB0002634946|10=0|");
    assert_eq!(text(&chain, 22).as_deref(), Some("4"));
    assert_eq!(
        text(&chain, yggdryl::ISINCODE_TAG_NAME.0).as_deref(),
        Some("GB0002634946")
    );
    assert_eq!(text(&chain, 470).as_deref(), Some("GB"));
    // The other way round, from the crate's column: `isincode` ->
    // `securityid` -> `securityidsource`, a lower tag filled off a higher.
    let reversed = settled(&reader, b"MSGTYPE=D|CLORDID=A|ISINCODE=GB0002634946");
    assert_eq!(text(&reversed, 48).as_deref(), Some("GB0002634946"));
    assert_eq!(text(&reversed, 22).as_deref(), Some("4"));
    // `cficode` -> `securitytype` -> `product`, and `exectype` ->
    // `ordstatus` -> `leavesqty` -> `state`.
    let typed = settled(&reader, b"8=FIX.4.4|35=D|11=A|461=LRXXXX|10=0|");
    assert_eq!(text(&typed, 167).as_deref(), Some("REPO"));
    assert_eq!(integer(&typed, 460), Some(13));
    let report = settled(&reader, b"8=FIX.4.4|35=8|150=0|38=100|14=0|10=0|");
    assert_eq!(text(&report, 39).as_deref(), Some("0"));
    assert_eq!(report.by_tag(151).unwrap(), Scalar::from(100.0_f64));
    assert_eq!(
        text(&report, yggdryl::STATE_TAG_NAME.0).as_deref(),
        Some(state("0").as_str())
    );
}

#[test]
fn every_shipped_derivation_is_canonical_and_binds_against_the_fields_it_reads() {
    // The dictionary stores each term as the grammar prints it, so what a
    // reader parses is byte for byte what a writer would store; and every
    // one binds against the registry's own fields for the columns it reads,
    // a group by its name, so no shipped rule is silent for want of a type.
    let registry = committed();
    let mut carried = 0;
    for field in registry.iter() {
        let Some(term) = field.as_fix().derivation().expect("readable") else {
            continue;
        };
        carried += 1;
        assert_eq!(
            field.get_metadata("fix:derivation"),
            Some(term.to_string().as_str()),
            "{} stores the canonical text",
            field.name()
        );
        let inputs: Vec<_> = term
            .columns()
            .iter()
            .map(|name| {
                registry
                    .get_field_by_name(name)
                    .or_else(|| registry.get_field_by_name(name))
                    .unwrap_or_else(|| {
                        panic!("{} reads {name}, which the dictionary names", field.name())
                    })
                    .clone()
            })
            .collect();
        let schema = DataType::from_fields(inputs)
            .expect("distinct columns")
            .required_field("row");
        term.bind(&schema).unwrap_or_else(|error| {
            panic!("{} binds against what it reads: {error}", field.name())
        });
    }
    // The dictionary's own rules and the crate's own columns.
    assert_eq!(carried, 42);
    for (tag, _) in [
        yggdryl::ISINCODE_TAG_NAME,
        yggdryl::MICCODE_TAG_NAME,
        yggdryl::STATE_TAG_NAME,
    ] {
        assert!(
            registry
                .field_by_tag(tag)
                .expect("a crate field")
                .as_fix()
                .derivation()
                .expect("readable")
                .is_some(),
            "tag {tag} derives"
        );
    }
    let _ = registry
        .get_field_by_name("secaltidgrp")
        .expect("the group a rule reads");
}

#[test]
fn the_crates_columns_fill_a_row_of_an_unenriched_message_as_the_pass_fills_it() {
    // The row door evaluates the crate columns' own derivations, the same
    // terms the pass evaluates, so a row of a message nobody enriched holds
    // what the pass would have stated.
    let reader = reader();
    let schema = yggdryl::fix_schema(reader.registry(), "fix").expect("the fixed schema");
    let line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|22=4|100=XNAS|150=F|10=0|";
    // A parse states the crate columns' own derivations, and the row door
    // evaluates the same terms, so the message and its row agree.
    let filled = reader.sole_line(line).expect("a readable line");
    let row = filled.clone().into_row(&schema).expect("a row");
    let column = |name: &str| {
        let at = schema.index_of(name).expect(name);
        row.as_sequence().expect("a row")[at].clone()
    };
    assert_eq!(column("isincode").as_str(), Some("US0378331005"));
    assert_eq!(column("miccode").as_str(), Some("XNAS"));
    assert_eq!(column("state").as_str(), Some(state("F").as_str()));
    assert_eq!(
        text(&filled, yggdryl::ISINCODE_TAG_NAME.0).as_deref(),
        Some("US0378331005")
    );
    assert_eq!(
        text(&filled, yggdryl::MICCODE_TAG_NAME.0).as_deref(),
        Some("XNAS")
    );
    assert_eq!(
        text(&filled, yggdryl::STATE_TAG_NAME.0).as_deref(),
        Some(state("F").as_str())
    );
}

/// A refusal's text, which names the field and what was refused.
fn refusal(error: &yggdryl::Error) -> String {
    error.to_string()
}

#[test]
fn a_malformed_derivation_in_a_store_or_a_snapshot_refuses_the_load_naming_the_field() {
    // The two load doors - a store on disk and a JSON snapshot - validate
    // the text as insert and update do: a stored `fix:derivation` that is
    // not a term refuses the whole load, naming the field that carries it.
    let mut registry = FixRegistry::new();
    let mut gross = DataType::Float64.nullable_field("grosstradeamt");
    gross.as_fix_mut().set_tag(381).expect("a tag");
    gross
        .as_fix_mut()
        .set_derivation(&"lastqty * lastpx".parse::<Term>().expect("a term"))
        .expect("stored");
    registry
        .insert(gross)
        .expect("a field carrying a derivation");

    let snapshot = registry.into_json().expect("a snapshot");
    assert!(
        FixRegistry::from_json(&snapshot).is_ok(),
        "the snapshot loads as written"
    );
    let corrupted = snapshot.replace("lastqty * lastpx", "lastqty *");
    assert_ne!(corrupted, snapshot, "the derivation is in the snapshot");
    let refused = FixRegistry::from_json(&corrupted).expect_err("refused at load");
    let rendered = refusal(&refused);
    assert!(rendered.contains("grosstradeamt"), "{rendered}");
    assert!(rendered.contains("fix:derivation"), "{rendered}");

    let root = Folder::temporary()
        .expect("a temporary folder")
        .path()
        .expect("a local path")
        .join(format!(
            "yggdryl-fix-derivation-load-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&root);
    let mut folder = Folder::new(&root).expect("a local folder");
    registry.write_into(&mut folder).expect("the store writes");
    assert!(
        FixRegistry::from_handle(&folder).is_ok(),
        "the store loads as written"
    );
    let mut corrupted = 0;
    for entry in std::fs::read_dir(root.join("fields")).expect("the fields shards") {
        let path = entry.expect("a shard").path();
        let text = std::fs::read_to_string(&path).expect("a shard is text");
        if text.contains("lastqty * lastpx") {
            std::fs::write(&path, text.replace("lastqty * lastpx", "lastqty *"))
                .expect("rewritten");
            corrupted += 1;
        }
    }
    assert_eq!(corrupted, 1, "one shard carries the derivation");
    let refused = FixRegistry::from_handle(&folder).expect_err("refused at load");
    let rendered = refusal(&refused);
    assert!(rendered.contains("grosstradeamt"), "{rendered}");
    assert!(rendered.contains("fix:derivation"), "{rendered}");
    std::fs::remove_dir_all(root).expect("cleaned up");
}

#[test]
fn a_registry_whose_derivations_do_not_compile_refuses_on_every_door() {
    // One derivation naming what the dictionary lacks: the line door, the
    // stream door, the row door and the batch door refuse alike, naming the
    // field, rather than one of them nulling the crate columns in silence.
    let mut registry = committed();
    let mut gross = registry.field_by_tag(381).expect("GrossTradeAmt").clone();
    gross
        .as_fix_mut()
        .set_derivation(&"lastqty * nosuchfield".parse::<Term>().expect("a term"))
        .expect("stored");
    registry.update(gross).expect("the text is a term");
    let reader = super::fixed_codec(Arc::new(registry));
    let schema = fix_schema(reader.registry(), "fix").expect("the fixed schema");
    let line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|22=4|100=XNAS|150=F|10=0|";
    // A parse is one of the doors that refuses, so the message the row and
    // the batch doors are handed is built rather than read.
    let root = DataType::from_fields([
        reader.registry().field_by_tag(37).expect("OrderID").clone(),
        reader.registry().field_by_tag(32).expect("LastQty").clone(),
    ])
    .expect("a struct root")
    .required_field("8");
    let value = Scalar::from_record([
        ("orderid", Scalar::from("A")),
        ("lastqty", Scalar::from(10.0_f64)),
    ])
    .expect("a record");
    let read =
        FixMsg::with_registry(Arc::clone(reader.registry()), root, value).expect("a built message");
    let names = |rendered: String| {
        assert!(rendered.contains("grosstradeamt"), "{rendered}");
        assert!(rendered.contains("nosuchfield"), "{rendered}");
    };
    names(refusal(
        &reader.sole_line(line).expect_err("the line door refuses"),
    ));
    names(refusal(
        &reader
            .parse_lines([line])
            .next()
            .expect("one item")
            .expect_err("the stream door refuses"),
    ));
    names(refusal(
        &read
            .clone()
            .into_row(&schema)
            .expect_err("the row door refuses"),
    ));
    names(
        reader
            .arrow_reader(schema, vec![read])
            .expect("a reader")
            .next()
            .expect("one item")
            .expect_err("the batch door refuses")
            .to_string(),
    );
}

#[test]
fn an_unvalidated_primary_under_the_isin_source_falls_through_to_the_alternate() {
    let reader = reader();
    let isincode = yggdryl::ISINCODE_TAG_NAME.0;
    // A primary no check digit closes, stated under the ISIN source beside
    // an alternate the digit does close: the alternate answers, and the
    // country is read off it.
    let fallen = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|22=4|48=NOTANISIN00|454=1|455=CH0012221716|456=4|10=0|",
    );
    assert_eq!(text(&fallen, isincode).as_deref(), Some("CH0012221716"));
    assert_eq!(text(&fallen, 470).as_deref(), Some("CH"));
    // A typo in the primary is the same fall-through: one digit off is not
    // that security.
    let typo = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|22=4|48=US0378331006|454=1|455=CH0012221716|456=4|10=0|",
    );
    assert_eq!(text(&typo, isincode).as_deref(), Some("CH0012221716"));
    // A primary the digit closes answers itself, whatever the alternate says.
    let primary = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|22=4|48=US0378331005|454=1|455=CH0012221716|456=4|10=0|",
    );
    assert_eq!(text(&primary, isincode).as_deref(), Some("US0378331005"));
    assert_eq!(text(&primary, 470).as_deref(), Some("US"));
    // Neither closes: silence, and nothing downstream reads a country.
    let neither = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|22=4|48=NOTANISIN00|454=1|455=CH0012221717|456=4|10=0|",
    );
    assert_eq!(neither.get_by_tag(isincode), None);
    assert_eq!(neither.get_by_tag(470), None);
}

#[test]
fn a_country_of_issue_is_exactly_a_prefix_the_crates_registry_lists() {
    // ISO 6166 opens a number with two letters, and `CountryOfIssue` answers
    // for exactly the pairs `StringEnum::COUNTRIES` lists: every one of the
    // 676 pairs, closed by its own check digit, answers its prefix where the
    // registry lists it and nothing where it does not.
    let reader = reader();
    let isincode = yggdryl::ISINCODE_TAG_NAME.0;
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
                text(&held, isincode).as_deref(),
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
    // Appendix D's cancel: a canceled order asked for what it did plus what
    // was canceled, and nothing is left. The old hand-laid order derived
    // `LeavesQty` first and answered `CumQty` alone; the rule now says it
    // outright, whether the report states nothing left or states nothing.
    let reader = reader();
    let closed = settled(&reader, b"8=FIX.4.4|35=8|37=A|39=4|14=40|84=60|10=0|");
    assert_eq!(closed.by_tag(38).unwrap(), Scalar::from(100.0_f64));
    assert_eq!(closed.by_tag(151).unwrap(), Scalar::from(0.0_f64));
    let stated = settled(&reader, b"8=FIX.4.4|35=8|37=A|39=4|14=40|151=0|84=60|10=0|");
    assert_eq!(stated.by_tag(38).unwrap(), Scalar::from(100.0_f64));
    let typed = settled(&reader, b"8=FIX.4.4|35=8|37=A|150=4|14=40|84=60|10=0|");
    assert_eq!(typed.by_tag(38).unwrap(), Scalar::from(100.0_f64));
    assert_eq!(text(&typed, 39).as_deref(), Some("4"));
    // A working report stating what is left orders done plus left, whatever
    // it canceled along the way: a replace that cut the quantity restated
    // what was ordered.
    let working = settled(
        &reader,
        b"8=FIX.4.4|35=8|37=A|39=1|14=40|151=40|84=20|10=0|",
    );
    assert_eq!(working.by_tag(38).unwrap(), Scalar::from(80.0_f64));
}

#[test]
fn the_fixpoint_reaches_the_chains_one_pass_could_not() {
    // Each of these fills a value the deleted single pass left absent - and
    // added on its second run, which is what made that pass non-idempotent.
    let reader = reader();
    // A forward price derived from spot and points is a price the worth
    // reads, and the average of one fill.
    let forward = settled(
        &reader,
        b"8=FIX.4.4|35=8|37=A|32=10|194=1.25|195=0.25|10=0|",
    );
    assert_eq!(forward.by_tag(31).unwrap(), Scalar::from(1.5_f64));
    assert_eq!(forward.by_tag(381).unwrap(), Scalar::from(15.0_f64));
    let average = settled(
        &reader,
        b"8=FIX.4.4|35=8|37=A|32=10|14=10|194=1.25|195=0.25|10=0|",
    );
    assert_eq!(average.by_tag(6).unwrap(), Scalar::from(1.5_f64));
    // What was ordered, read off what was canceled, is what the remainder
    // reads: a new order that canceled nothing yet has everything left.
    let fresh = settled(&reader, b"8=FIX.4.4|35=8|37=A|150=0|14=0|84=100|10=0|");
    assert_eq!(fresh.by_tag(38).unwrap(), Scalar::from(100.0_f64));
    assert_eq!(text(&fresh, 39).as_deref(), Some("0"));
    assert_eq!(fresh.by_tag(151).unwrap(), Scalar::from(100.0_f64));
    // A trade over several periods, then the multiplied and the gross
    // quantities read off it.
    let periods = settled(
        &reader,
        b"8=FIX.4.4|35=D|11=A|32=10|31=3|2353=2|231=5|10=0|",
    );
    assert_eq!(periods.by_tag(2367).unwrap(), Scalar::from(20.0_f64));
    assert_eq!(periods.by_tag(2370).unwrap(), Scalar::from(100.0_f64));
    assert_eq!(periods.by_tag(2369).unwrap(), Scalar::from(60.0_f64));
    assert_eq!(periods.by_tag(2368).unwrap(), Scalar::from(50.0_f64));
}

#[test]
fn a_typed_read_fires_where_the_old_text_read_could_not() {
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
    assert_eq!(periods.by_tag(2367).unwrap(), Scalar::from(20.0_f64));
}

#[test]
fn a_source_code_is_compared_exactly_because_fix_codes_are_case_sensitive() {
    // `A` is Bloomberg's source; `a` is no code of the set, and the deleted
    // rule's case folding read it as one.
    let reader = reader();
    let bloomberg = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=ABBN SW|22=A|10=0|");
    assert_eq!(text(&bloomberg, 55).as_deref(), Some("ABBN SW"));
    let folded = settled(&reader, b"8=FIX.4.4|35=D|11=A|48=ABBN SW|22=a|10=0|");
    assert_eq!(folded.get_by_tag(55), None);
}

#[test]
fn a_registry_of_a_handful_of_fields_enriches_and_its_crate_columns_are_silent() {
    // The crate columns' terms read the standard's fields by name; a
    // registry holding none of them widens them as columns no message
    // states, and a message enriches without a refusal and without a fill.
    let registry = FixRegistry::new();
    let reader = super::fixed_codec(Arc::new(registry));
    let read = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|48=US0378331005|22=4|207=XNAS|10=0|")
        .expect("a readable line");
    let held = read;
    for (tag, _) in [
        yggdryl::ISINCODE_TAG_NAME,
        yggdryl::MICCODE_TAG_NAME,
        yggdryl::STATE_TAG_NAME,
    ] {
        assert!(
            held.get_by_tag(tag).is_none_or(|held| held.is_null()),
            "tag {tag} is silent"
        );
    }
}

#[test]
fn a_stream_of_every_shape_costs_nothing_between_messages() {
    // No shape is recognized and nothing is kept per shape or between
    // messages: the bridge's own capture, every shape it writes, enriches
    // to the same answers message by message in either order, through the
    // stream door twice, over what the stream already enriched - and the
    // stream answers exactly what the one-message door answers, because it
    // carries nothing from one message to the next.
    let reader = super::fixed_codec(super::committed_registry());
    let source = Buffer::from_bytes(include_bytes!("ulbridge.log").to_vec()).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    );
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
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
    // and never none.
    assert_eq!(messages.len(), 94, "the corpus");
    let forward: Vec<FixMsg> = messages.iter().map(|message| message.clone()).collect();
    let mut backward: Vec<FixMsg> = messages
        .iter()
        .rev()
        .map(|message| message.clone())
        .collect();
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
