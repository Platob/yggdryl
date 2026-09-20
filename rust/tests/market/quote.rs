//! A quote: a price stated at an instant, one or two lanes under the
//! quote's own identifiers and its validity, its cancel ending the chain,
//! and no message that states it back.

use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::market::{Product, Quote, QuoteData};
use yggdryl::{Decimal18, FixMsg};

use super::{codec, message_reader, parsed, product_rows, rows_of};

/// A two-sided quote, its update and its cancel; then a one-sided quote of
/// another identifier.
const QUOTES: [&[u8]; 4] = [
    b"8=FIX.4.4|35=S|117=Q1|131=R1|55=AAPL|132=10.4|134=100|133=10.6|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:30.250|10=0|",
    b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=100|133=10.55|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:31.250|10=0|",
    b"8=FIX.4.4|35=Z|117=Q1|298=1|52=20260102-10:15:32.250|10=0|",
    b"8=FIX.4.4|35=S|117=Q2|55=AAPL|54=1|132=10.3|134=50|15=USD|52=20260102-10:15:33.250|10=0|",
];

fn decimal(text: &str) -> Decimal18 {
    text.parse().expect("a number")
}

#[test]
fn a_quote_is_its_lanes_under_its_identifiers_and_its_validity() {
    let codec = codec();
    let quotes: Vec<QuoteData> = codec
        .quotes(parsed(&codec, &QUOTES))
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    assert_eq!(quotes.len(), 4);
    let [first, update, cancel, one_sided] = quotes.as_slice() else {
        panic!("four statements")
    };
    assert_eq!(first.get_bidpx(), Some(decimal("10.4")));
    assert_eq!(first.get_bidqty(), Some(Decimal18::from_int(100)));
    assert_eq!(first.get_askpx(), Some(decimal("10.6")));
    assert_eq!(first.get_askqty(), Some(Decimal18::from_int(150)));
    assert_eq!(
        first.get_bidcurrency().map(yggdryl::Currency::as_str),
        Some("USD"),
        "a lane is priced in the quote's currency"
    );
    assert_eq!(
        first.get_askcurrency().map(yggdryl::Currency::as_str),
        Some("USD")
    );
    assert_eq!(first.get_expirunix(), Some(1_767_348_960_000_000_000));
    assert_eq!(first.get_crosscode(), "Q1");
    assert_eq!(first.get_identifiers()["quoteid"], "Q1");
    assert_eq!(first.get_identifiers()["quotereqid"], "R1");
    // What the lanes imply.
    assert!(first.is_two_sided());
    assert_eq!(
        first.bid(),
        Some((decimal("10.4"), Decimal18::from_int(100)))
    );
    assert_eq!(first.mid(), Some(decimal("10.5")));
    assert_eq!(first.spread(), Some(decimal("0.2")));
    assert_eq!(
        first.lane(&yggdryl::Side::read("Sell").expect("a side")),
        first.ask()
    );
    // The update follows under the quote's identifier, and the cancel ends
    // the chain.
    assert_eq!(update.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(update.get_bidpx(), Some(decimal("10.45")));
    assert_eq!(
        update.get_prevpx(),
        None,
        "a two-sided quote is about no one price"
    );
    assert_eq!(cancel.get_seqnum(), 2);
    assert_eq!(cancel.get_state().as_str(), "90CANCELED");
    assert!(!cancel.get_state().is_live());
    assert_eq!(cancel.get_bidpx(), None);
    // A one-sided quote fills the lane its side implies and no other.
    assert_eq!(one_sided.get_crosscode(), "Q2");
    assert_eq!(one_sided.get_seqnum(), 0);
    assert_eq!(one_sided.get_bidpx(), Some(decimal("10.3")));
    assert_eq!(one_sided.get_askpx(), None);
    assert_eq!(
        one_sided.get_px(),
        decimal("10.3"),
        "about the bid it states"
    );
    assert!(!one_sided.is_two_sided());
    assert_eq!((one_sided.mid(), one_sided.spread()), (None, None));
    assert_eq!(cancel.bid(), None, "a cancel quotes no lane");
    // Each names the message it was read from.
    let chained: Vec<FixMsg> = codec
        .lifecycle(parsed(&codec, &QUOTES))
        .collect::<yggdryl::Result<_>>()
        .expect("the lifecycle");
    for (quote, message) in quotes.iter().zip(&chained) {
        assert_eq!(quote.get_srcuuids(), [message.get_curruuid()]);
    }
}

#[test]
fn the_row_round_trips_and_the_arrow_door_agrees() {
    let codec = codec();
    let field = QuoteData::field().expect("the quote row");
    let names: Vec<&str> = field.fields()[16..]
        .iter()
        .map(yggdryl::Field::name)
        .collect();
    assert_eq!(
        &names[..8],
        &[
            "bidpx",
            "bidqty",
            "bidcurrency",
            "bidunit",
            "askpx",
            "askqty",
            "askcurrency",
            "askunit"
        ]
    );
    let messages = parsed(&codec, &QUOTES);
    let quotes: Vec<QuoteData> = codec
        .quotes(messages.clone())
        .collect::<yggdryl::Result<_>>()
        .expect("every message reads");
    for quote in &quotes {
        let row = quote.into_row().expect("a row");
        let again = QuoteData::from_row(&field, &row).expect("reads");
        assert_eq!(again.into_row().expect("a row"), row);
        assert_eq!(again.get_curruuid(), quote.get_curruuid());
    }
    let rows = rows_of(
        codec
            .quotes_arrow_reader(message_reader(&codec, messages.clone()))
            .expect("the quote rows open"),
    );
    assert_eq!(rows, product_rows(codec.quotes(messages)));
    assert_eq!(rows.len(), 4);
}

#[test]
fn no_message_states_a_quote_and_the_refusal_says_why() {
    let codec = codec();
    let quote = codec
        .quotes(parsed(&codec, &QUOTES))
        .next()
        .expect("a quote")
        .expect("it reads");
    let refused = FixMsg::from_quote(&codec, &quote).unwrap_err().to_string();
    assert!(
        refused.contains("quote") && refused.contains("does not guess"),
        "{refused}"
    );
}
