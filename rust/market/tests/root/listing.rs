//! `rust/market/src/listing.rs`: one listing of an instrument - its market,
//! its ticker, its trading currency and its listing codes - holding listing
//! codes alone, bounded, crossing its scalar and reading back.

use yggdryl::{Ccy, Mic, Scalar};
use yggdryl_market::{IdType, Instrument, Listing};

fn mic(text: &str) -> Option<Mic> {
    Some(Mic::new(text).unwrap())
}

/// A listing holds listing codes alone, each held as its type stores it,
/// at most the bound, a ticker trimmed to one to sixty-four bytes and a
/// currency other than `XXX`; `XXXX` is no market.
#[test]
fn a_listing_holds_listing_codes_alone_and_trims_its_ticker() {
    crate::install::installed();
    let listing = Listing::new(mic("XSWX"))
        .with_ticker(Some("  HOLN ".into()))
        .with_currency(Some(Ccy::new("CHF").unwrap()))
        .try_with_code(IdType::Ric, "HOLN.S")
        .unwrap()
        .try_with_code(IdType::Bloomberg, "HOLN SW Equity")
        .unwrap();
    assert_eq!(listing.miccode().map(Mic::as_str), Some("XSWX"));
    assert_eq!(listing.ticker(), Some("HOLN"));
    assert_eq!(listing.currency().map(Ccy::as_str), Some("CHF"));
    assert_eq!(listing.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(listing.codes().len(), 2);
    assert!(
        listing
            .clone()
            .try_with_code(IdType::ExchSymb, "HOLN")
            .is_err(),
        "past the bound of {}",
        Instrument::MAX_LISTING_CODES
    );
    assert!(
        listing
            .clone()
            .try_with_code(IdType::Ric, "HOLN.VX")
            .is_ok(),
        "a held type restated"
    );
    for refused in [IdType::Cusip, IdType::Isin, IdType::Lei, IdType::Common] {
        let error = listing
            .clone()
            .try_with_code(refused.clone(), "X")
            .unwrap_err()
            .to_string();
        assert!(error.contains("listing code"), "{refused}: {error}");
    }
    assert!(
        Listing::new(mic("XSWX"))
            .try_with_code(IdType::Ric, "")
            .is_err()
    );
    assert_eq!(Listing::new(mic("XXXX")).miccode(), None, "no market");
    assert_eq!(Listing::new(None).miccode(), None);
    let long = "T".repeat(65);
    assert_eq!(
        Listing::new(None).with_ticker(Some(long.into())).ticker(),
        None
    );
    assert_eq!(
        Listing::new(None).with_ticker(Some("   ".into())).ticker(),
        None
    );
    assert_eq!(
        Listing::new(None)
            .with_currency(Some(Ccy::new("XXX").unwrap()))
            .currency(),
        None
    );
}

/// A listing crosses its scalar - the named struct of four cells, no codes
/// a null - and reads back from the named struct, a name it lacks a null,
/// and from the ordered row its field canonicalizes it to.
#[test]
fn a_listing_crosses_its_scalar_and_reads_back() {
    crate::install::installed();
    let listing = Listing::new(mic("XLON"))
        .with_ticker(Some("HSBA".into()))
        .with_currency(Some(Ccy::new("GBP").unwrap()))
        .try_with_code(IdType::Ric, "HSBA.L")
        .unwrap();
    let scalar = listing.into_scalar();
    let cells = scalar.as_struct().unwrap();
    assert_eq!(cells.get("miccode").and_then(Scalar::as_str), Some("XLON"));
    assert_eq!(cells.get("ticker").and_then(Scalar::as_str), Some("HSBA"));
    assert!(cells.get("codes").unwrap().as_mapping().is_some());
    assert_eq!(Listing::from_scalar(&scalar).unwrap(), listing);
    let row = Listing::field().scalar(scalar).unwrap();
    assert!(row.as_struct().is_none(), "canonical: an ordered row");
    assert_eq!(Listing::from_scalar(&row).unwrap(), listing);
    let bare = Listing::new(None);
    assert_eq!(
        bare.into_scalar().as_struct().unwrap().get("codes"),
        Some(&Scalar::Null)
    );
    assert_eq!(Listing::from_scalar(&bare.into_scalar()).unwrap(), bare);
    let named = Scalar::from_struct([("miccode", Scalar::from("XNAS"))]).unwrap();
    assert_eq!(
        Listing::from_scalar(&named).unwrap(),
        Listing::new(mic("XNAS"))
    );
    // A code that is no listing code is refused where it is read.
    let refused = Scalar::from_struct([
        ("miccode", Scalar::from("XNAS")),
        (
            "codes",
            Scalar::from_mapping([(Scalar::from("cusip"), Scalar::from("037833100"))]).unwrap(),
        ),
    ])
    .unwrap();
    assert!(Listing::from_scalar(&refused).is_err());
    let dtype = Listing::dtype();
    let names: Vec<&str> = dtype
        .as_fields()
        .unwrap()
        .iter()
        .map(|field| field.name())
        .collect();
    assert_eq!(names, ["miccode", "ticker", "currency", "codes"]);
    assert!(!Listing::field().is_nullable());
    assert_eq!(Listing::MAX_TICKER_WIDTH, 64);
}
