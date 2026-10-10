//! `rust/fix/src/forex.rs`: the currency pair a message's symbol names,
//! detected where the message states no other class, and the cells it fills.

use std::sync::Arc;

use super::SoleMessage;
use yggdryl::{Cfi, Scalar};
use yggdryl_fix::{FixCodec, FixMsg, FixRegistry, fix_schema};
use yggdryl_market::graph::Market;
use yggdryl_market::{IdKey, IdType, Identifier};

fn reader() -> (Arc<FixRegistry>, FixCodec) {
    let registry = super::committed_registry();
    let reader = super::fixed_codec(Arc::clone(&registry));
    (registry, reader)
}

/// What the message states under `tag`, as text: a string or a code as it
/// is spelled, an integer as its digits; `None` for a null or no cell.
fn cell(message: &FixMsg, tag: i32) -> Option<String> {
    let held = message.get_by_tag(tag)?;
    held.as_str()
        .map(ToOwned::to_owned)
        .or_else(|| held.as_i64().map(|value| value.to_string()))
}

fn pair(code: &str) -> Identifier {
    Identifier::new(IdKey::base(IdType::Forex), code).expect("a pair")
}

/// The pair the message's identifiers hold under `forex`.
fn forex(message: &FixMsg) -> Option<String> {
    message
        .get_securityids()
        .get(&IdType::Forex)
        .map(ToOwned::to_owned)
}

/// Whether the message holds its pair as derived: a stated pair is kept
/// against another, a derived one is replaced.
fn derives_its_pair(message: &FixMsg) -> bool {
    message
        .clone()
        .insert_securityid(pair("GBP/JPY"))
        .expect("a pair the message can state")
}

#[test]
fn a_stated_class_that_is_not_foreign_exchange_blocks_every_fill() {
    crate::install::installed();
    let (_, reader) = reader();
    for line in [
        &b"8=FIX.4.4|35=D|11=A|55=EUR/USD|167=CS|10=0|"[..],
        b"8=FIX.4.4|35=D|11=A|55=EUR/USD|460=5|10=0|",
        b"8=FIX.4.4|35=D|11=A|55=EUR/USD|461=ESVUFR|10=0|",
        // A metal's commodity product and spot-commodity CFI admit only a
        // metal.
        b"8=FIX.4.4|35=D|11=A|55=EUR/USD|460=2|10=0|",
        b"8=FIX.4.4|35=D|11=A|55=EUR/USD|461=ITKXXX|10=0|",
    ] {
        let message = reader.sole_line(line).expect("an order");
        let spelled = String::from_utf8_lossy(line);
        assert_eq!(forex(&message), None, "{spelled}");
        assert_eq!(cell(&message, 65_049), None, "{spelled}");
        assert_ne!(cell(&message, 461).as_deref(), Some("IFXXXP"), "{spelled}");
        assert_eq!(cell(&message, 15), None, "{spelled}");
        assert_eq!(cell(&message, 120), None, "{spelled}");
    }
    // A symbol naming no pair detects nothing, and neither does none.
    for line in [
        &b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|"[..],
        b"8=FIX.4.4|35=D|11=A|55=EUR=|10=0|",
        b"8=FIX.4.4|35=D|11=A|55=[N/A]|10=0|",
        b"8=FIX.4.4|35=D|11=A|10=0|",
    ] {
        let message = reader.sole_line(line).expect("an order");
        assert_eq!(forex(&message), None);
        assert_eq!(cell(&message, 167), None);
        assert_eq!(cell(&message, 460), None);
    }
}

#[test]
fn a_stated_currency_that_is_neither_leg_leaves_both_currencies() {
    crate::install::installed();
    let (_, reader) = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=EUR/USD|15=GBP|10=0|")
        .expect("an order");
    assert_eq!(forex(&message).as_deref(), Some("EUR/USD"));
    assert_eq!(cell(&message, 15).as_deref(), Some("GBP"));
    assert_ne!(cell(&message, 120).as_deref(), Some("USD"));
    assert_ne!(cell(&message, 120).as_deref(), Some("EUR"));
    // A stated currency that is the quote leg settles in the base.
    let quoted = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=EUR/USD|15=USD|10=0|")
        .expect("an order");
    assert_eq!(cell(&quoted, 15).as_deref(), Some("USD"));
    assert_eq!(cell(&quoted, 120).as_deref(), Some("EUR"));
}

#[test]
fn a_spot_pair_states_its_class_its_currencies_and_no_market() {
    crate::install::installed();
    let (_, reader) = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=EUR/USD|54=1|38=1000000|10=0|")
        .expect("an order");
    assert_eq!(forex(&message).as_deref(), Some("EUR/USD"));
    assert!(
        derives_its_pair(&message),
        "detected is derived, not stated"
    );
    assert_eq!(
        message.get_by_tag(65_049),
        Some(Scalar::Forex(yggdryl::Forex::new("EUR/USD").unwrap()))
    );
    assert_eq!(cell(&message, 167).as_deref(), Some("FXSPOT"));
    assert_eq!(cell(&message, 460).as_deref(), Some("4"));
    assert_eq!(cell(&message, 461).as_deref(), Some("IFXXXP"));
    assert_eq!(cell(&message, 15).as_deref(), Some("EUR"));
    assert_eq!(cell(&message, 120).as_deref(), Some("USD"));
    assert_eq!(message.get_currency().as_str(), "EUR");
    assert_eq!(
        message.get_cficode().map(|held| held.as_str()),
        Some("IFXXXP")
    );
    assert_eq!(
        message.get_miccode().map(|held| held.as_str()),
        Some("XXXX"),
        "a pair trades on no one market"
    );
    // A spot symbol states no settlement type of its own.
    assert_eq!(cell(&message, 63), None);
    // The pair keys the instrument `IF:EUR/USD` - the `instcode` its book
    // is keyed by (the FX book took its code, decision 16) - whose number
    // the parse mints into the derived overlay beside the pair (D42): the
    // row's `isincode`, under a stated ISIN whatever its rank.
    assert_eq!(message.get_instcode(), Some("IF:EUR/USD"));
    assert_eq!(message.get_isincode(), Some("QYLTVIRYHNX5"));
    assert!(message.get_securityids().is_derived(&IdType::Isin));
    let numbered = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=EUR/USD|22=4|48=EZ0000000003|54=1|38=1000000|10=0|")
        .expect("an order");
    assert_eq!(numbered.get_instcode(), Some("IF:EUR/USD"));
    assert_eq!(
        numbered.get_isincode(),
        Some("EZ0000000003"),
        "a stated ISIN wins over the mint, rank two over one"
    );
    assert!(!numbered.get_securityids().is_derived(&IdType::Isin));
    // Every spelling of the pair is the one pair.
    for symbol in [
        "EURUSD",
        "eur-usd",
        "EURUSD=",
        "EUR/USD SPOT",
        "EURUSD CURNCY",
    ] {
        let line = format!("8=FIX.4.4|35=D|11=A|55={symbol}|10=0|");
        let held = reader.sole_line(line.as_bytes()).expect("an order");
        assert_eq!(forex(&held).as_deref(), Some("EUR/USD"), "{symbol}");
        assert_eq!(cell(&held, 167).as_deref(), Some("FXSPOT"), "{symbol}");
    }
}

#[test]
fn a_forward_pair_is_an_fx_forward_settling_on_its_tenor() {
    crate::install::installed();
    let (_, reader) = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=EUR/USD 1M|10=0|")
        .expect("an order");
    assert_eq!(forex(&message).as_deref(), Some("EUR/USD"));
    assert_eq!(cell(&message, 167).as_deref(), Some("FXFWD"));
    assert_eq!(cell(&message, 461).as_deref(), Some("JFTXFP"));
    assert_eq!(cell(&message, 63).as_deref(), Some("M1"));
    // The forward is keyed by its pair and its tenor, and minted its own
    // number (D42); a stated settlement date keys it by the date instead,
    // and leaves the type to the message.
    assert_eq!(message.get_instcode(), Some("JF:EUR/USD:M1"));
    assert_eq!(
        message.get_isincode(),
        Some(yggdryl_market::Instrument::minted_number("JF:EUR/USD:M1").as_str())
    );
    let dated = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=EUR/USD 1M|64=20240202|10=0|")
        .expect("an order");
    assert_eq!(cell(&dated, 63), None);
    assert_eq!(dated.get_instcode(), Some("JF:EUR/USD:2024-02-02"));
    let (class, pair, characteristics) = dated.stated_characteristics().expect("a class");
    assert_eq!(class.as_str(), "JFTXFP");
    assert_eq!(
        pair.map(|pair| pair.to_string()),
        Some("EUR/USD".to_owned())
    );
    assert_eq!(
        characteristics.settle().map(ToString::to_string),
        Some("2024-02-02".to_owned())
    );
}

#[test]
fn a_metal_pair_is_a_commodity_with_no_security_type() {
    crate::install::installed();
    // The spot-commodity group takes a precious metal at position 3.
    assert!(Cfi::is_detailed("ITKXXX"));
    let (_, reader) = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=XAU/USD|10=0|")
        .expect("an order");
    assert_eq!(forex(&message).as_deref(), Some("XAU/USD"));
    assert_eq!(cell(&message, 460).as_deref(), Some("2"));
    assert_eq!(cell(&message, 461).as_deref(), Some("ITKXXX"));
    assert_ne!(cell(&message, 167).as_deref(), Some("FXSPOT"));
    assert_eq!(cell(&message, 15).as_deref(), Some("XAU"));
    assert_eq!(cell(&message, 120).as_deref(), Some("USD"));
}

#[test]
fn a_stated_identifier_beside_the_pair_keeps_both() {
    crate::install::installed();
    let (_, reader) = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=EUR/USD|22=4|48=CH0012221716|10=0|")
        .expect("an order");
    assert_eq!(
        message.get_securityids().get(&IdType::Isin),
        Some("CH0012221716")
    );
    assert_eq!(forex(&message).as_deref(), Some("EUR/USD"));
}

#[test]
fn a_detected_row_reads_back_stating_its_pair() {
    crate::install::installed();
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=EUR/USD|54=1|38=100|44=1.0850|10=0|")
        .expect("an order");
    let row = message.into_row(&schema).unwrap();
    let mut back = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(back.into_row(&schema).unwrap(), row, "a fixed point");
    assert_eq!(forex(&back).as_deref(), Some("EUR/USD"));
    // The row states where its pair came from: the identifier's source is
    // part of the row, so the pair detection derived reads back derived.
    assert!(back.get_securityids().is_derived(&IdType::Forex));
    // A changed symbol leaves a pair the row stated.
    back.set(55, Scalar::from("GBP/USD")).unwrap();
    assert_eq!(forex(&back).as_deref(), Some("EUR/USD"));
    assert_eq!(cell(&back, 15).as_deref(), Some("EUR"));
}

#[test]
fn a_changed_symbol_derives_again_and_keeps_what_a_caller_set() {
    crate::install::installed();
    let (_, reader) = reader();
    let mut message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=EUR/USD|10=0|")
        .expect("an order");
    assert_eq!(cell(&message, 167).as_deref(), Some("FXSPOT"));
    message.set(167, Scalar::from("FXFWD")).unwrap();
    message.set(55, Scalar::from("GBP/USD")).unwrap();
    assert_eq!(forex(&message).as_deref(), Some("GBP/USD"));
    assert_eq!(
        cell(&message, 65_049).as_deref(),
        Some("GBP/USD"),
        "the cell follows the pair"
    );
    assert_eq!(cell(&message, 15).as_deref(), Some("GBP"));
    assert_eq!(cell(&message, 120).as_deref(), Some("USD"));
    assert_eq!(
        cell(&message, 167).as_deref(),
        Some("FXFWD"),
        "the caller's"
    );
    assert_eq!(
        cell(&message, 461).as_deref(),
        Some("JFTXFP"),
        "detection's own cell follows the caller's class"
    );
    // A symbol naming no pair takes back everything detection derived.
    message.set(55, Scalar::from("AAPL")).unwrap();
    assert_eq!(forex(&message), None);
    assert_eq!(cell(&message, 65_049), None);
    assert_eq!(cell(&message, 15), None);
    assert_eq!(cell(&message, 120), None);
    assert_eq!(cell(&message, 460), None);
    assert_eq!(
        cell(&message, 167).as_deref(),
        Some("FXFWD"),
        "the caller's"
    );
}

#[test]
fn a_disagreeing_spelling_of_the_stated_pair_is_an_anomaly() {
    crate::install::installed();
    let (_, reader) = reader();
    let message = reader
        .sole_line(b"MSGTYPE=D|CLORDID=A|SYMBOL=EUR/USD|FOREXCODE=EUR/USD|CCYPAIR=GBP/USD")
        .expect("a bridge row");
    // The bridge's own pair is read with the fields, so it states the
    // type's answer first; the view disagreeing with it is dropped.
    assert_eq!(
        forex(&message).as_deref(),
        Some("GBP/USD"),
        "the fields' reading wins"
    );
    let anomalies: Vec<String> = message
        .anomalies()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        anomalies
            .iter()
            .any(|held| held.contains("states forex=EUR/USD where forex=GBP/USD")),
        "{anomalies:?}"
    );
}

#[test]
fn a_stated_pair_is_its_column_and_never_an_alternate_identifier() {
    crate::install::installed();
    let (_, reader) = reader();
    let alternates = |message: &FixMsg| {
        message
            .get_by_name("secaltids")
            .map(|held| format!("{held:?}"))
            .unwrap_or_default()
    };
    let mut message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|454=1|455=AAPL.O|456=5|10=0|")
        .expect("an order");
    let before = alternates(&message);
    assert!(message.insert_securityid(pair("EUR/USD")).unwrap());
    assert_eq!(alternates(&message), before, "no forex occurrence");
    assert_eq!(cell(&message, 65_049).as_deref(), Some("EUR/USD"));
    assert!(!derives_its_pair(&message));

    // A successor inherits its predecessor's pair the same way.
    let first = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|454=1|455=AAPL.O|456=5|10=0|")
        .expect("an order");
    let mut first = first;
    first.insert_securityid(pair("EUR/USD")).unwrap();
    let second = reader
        .sole_line(b"8=FIX.4.4|35=8|11=A|17=E1|150=0|39=0|454=1|455=AAPL.O|456=5|10=0|")
        .expect("a report");
    let before = alternates(&second);
    let walked = reader
        .lifecycle([first, second])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(walked.len(), 2);
    assert_eq!(forex(&walked[1]).as_deref(), Some("EUR/USD"));
    assert_eq!(alternates(&walked[1]), before, "no forex occurrence");
}

#[cfg(feature = "internals")]
mod internal {
    use super::SoleMessage;
    use yggdryl_fix::internals::forex::{derive, memo, parses, remembered, symbol};

    /// Detection over a message naming no pair allocates nothing once its
    /// symbol is remembered, and neither does a detected message detected
    /// again: a hit clones inline texts and every cell already holds what
    /// detection would write. Pinned at two repeat counts, so a per-call
    /// allocation cannot hide in a constant.
    #[test]
    fn a_remembered_symbol_detects_without_allocating() {
        crate::install::installed();
        let (_, reader) = super::reader();
        let held = memo();
        for line in [
            &b"8=FIX.4.4|35=D|11=A|55=AAPL|54=1|38=100|10=0|"[..],
            b"8=FIX.4.4|35=D|11=A|55=EUR/USD|54=1|38=100|10=0|",
        ] {
            let mut message = reader.sole_line(line).expect("an order");
            derive(&mut message, &held).expect("detects");
            for repeats in [1, 16] {
                let ((), counts) = crate::allocations::measure(|| {
                    for _ in 0..repeats {
                        derive(&mut message, &held).expect("detects");
                    }
                });
                assert_eq!(
                    counts.allocations,
                    0,
                    "{} x{repeats}",
                    String::from_utf8_lossy(line)
                );
            }
        }
    }

    #[test]
    fn a_symbol_is_parsed_once_and_answered_after() {
        crate::install::installed();
        let held = memo();
        let first = symbol(&held, "EUR/USD 1M").expect("a pair");
        assert_eq!(parses(&held), 1);
        assert_eq!(symbol(&held, " EUR/USD 1M "), Some(first));
        assert_eq!(parses(&held), 1, "a hit parses nothing");
        assert_eq!(symbol(&held, "AAPL"), None);
        assert_eq!(symbol(&held, "AAPL"), None);
        assert_eq!(parses(&held), 2, "a refusal is remembered too");
    }

    #[test]
    fn past_the_bound_a_symbol_is_answered_and_not_remembered() {
        crate::install::installed();
        let held = memo();
        let bound = yggdryl_fix::internals::forex::FxMemo::CAPACITY;
        for index in 0..bound {
            assert_eq!(symbol(&held, &format!("S{index}")), None);
        }
        assert_eq!(remembered(&held), bound);
        let past = parses(&held);
        assert_eq!(
            symbol(&held, "EUR/USD").map(|held| held.forex.to_string()),
            Some("EUR/USD".to_owned()),
            "answered"
        );
        assert_eq!(remembered(&held), bound, "not remembered");
        let _ = symbol(&held, "EUR/USD");
        assert_eq!(parses(&held), past + 2, "parsed again");
    }
}
