//! `rust/market/src/instrument/seed.rs`: the seed every process default starts
//! from - `config/instruments/instruments.json`, embedded at build time through the
//! crate's copy `rust/market/src/instrument/seed.json` - read as ordinary
//! statements, one object per instrument with its listings nested, so what
//! the embedded copy holds is pinned here and the data cannot rot silently.

use std::collections::BTreeMap;

use smol_str::SmolStr;
use yggdryl::graph::Element;
use yggdryl::{Mic, Scalar};
use yggdryl_market::{IdType, Instrument, Instruments, Listing};

/// The text the crate embeds: its copy of the seed, which its package
/// carries.
const EMBEDDED: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/instrument/seed.json"
));

/// The embedded seed document: one named row per instrument.
fn document() -> Vec<BTreeMap<SmolStr, Scalar>> {
    yggdryl::from_json_scalar(EMBEDDED)
        .expect("a JSON document")
        .sequence_rows()
        .expect("a JSON array")
        .iter()
        .map(|row| row.as_struct().expect("an object").clone())
        .collect()
}

/// The text `row` states under `key`.
fn cell<'row>(row: &'row BTreeMap<SmolStr, Scalar>, key: &str) -> Option<&'row str> {
    row.get(key).and_then(Scalar::as_str)
}

/// The listings `row` states, each a named struct.
fn listings(row: &BTreeMap<SmolStr, Scalar>) -> Vec<BTreeMap<SmolStr, Scalar>> {
    row.get("listings")
        .and_then(Scalar::sequence_rows)
        .map(|rows| {
            rows.iter()
                .map(|listing| listing.as_struct().expect("an object").clone())
                .collect()
        })
        .unwrap_or_default()
}

fn mic(text: &str) -> Mic {
    Mic::new(text).unwrap()
}

/// The crate's copy is `config/instruments/instruments.json` byte for byte, where
/// the repository holds that file - a checkout, never the published crate,
/// which carries the copy alone: `python scripts/check_instruments_seed.py --sync`
/// writes it.
#[test]
fn the_embedded_seed_is_the_config_file_byte_for_byte() {
    crate::install::installed();
    let config = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("config")
        .join("instruments")
        .join("instruments.json");
    if !config.is_file() {
        println!("SKIPPED: no {} beside the crate", config.display());
        return;
    }
    let held = std::fs::read(&config).expect("the seed file");
    assert!(
        held == EMBEDDED.as_bytes(),
        "rust/market/src/instrument/seed.json differs from config/instruments/instruments.json: run python scripts/check_instruments_seed.py --sync"
    );
}

/// Every object of the document reaches the seed once, as the instrument
/// keyed by its real ISIN, in cross code order, with every value it states,
/// its listings nested, one per market in MIC order: none is refused or
/// lands null.
#[test]
fn the_seed_holds_every_instrument_of_the_document_once_in_code_order() {
    crate::install::installed();
    let rows = document();
    let seeded = Instruments::seeded();
    assert_eq!(rows.len(), 208);
    assert_eq!(seeded.len(), 208);
    assert_eq!(seeded.rows(), 208, "one row per instrument");
    assert_eq!(
        rows.iter().map(|row| listings(row).len()).sum::<usize>(),
        209,
        "HSBC on two markets"
    );
    let stated: Vec<&str> = rows.iter().map(|row| cell(row, "isin").unwrap()).collect();
    let held: Vec<&str> = seeded.iter().map(Element::get_crosscode).collect();
    assert_eq!(held, stated, "unique and sorted by the key, the ISIN");
    assert_eq!(held.first(), Some(&"AU000000BHP4"));
    assert_eq!(held.last(), Some(&"XC0006013624"));
    for instrument in seeded.iter() {
        assert_eq!(instrument.isin(), Some(instrument.get_crosscode()));
        assert!(!instrument.is_placeholder());
        assert_eq!(instrument.minted_isin(), None, "an agency numbered it");
        assert_eq!(instrument.get_uuid(), instrument.get_crossuuid());
    }
    for row in &rows {
        let isin = cell(row, "isin").unwrap();
        let entry = seeded.get(isin).unwrap();
        assert_eq!(
            entry.cficode().as_ref().map(|code| code.as_str()),
            cell(row, "cficode"),
            "{isin}"
        );
        assert_eq!(
            entry.fisn().as_ref().map(yggdryl::Fisn::as_str),
            cell(row, "fisn"),
            "{isin}"
        );
        assert_eq!(
            entry.origccy().map(|code| code.as_str()),
            cell(row, "origccy"),
            "{isin}: the origin currency stated, and none derived"
        );
        // A listed country is the instrument's country; the ISIN's own
        // prefix is held as none beside it, and a reserved code (`EZ`) is
        // no country.
        let country = cell(row, "countrycode")
            .filter(|code| yggdryl::Country::new(*code).unwrap().is_listed());
        assert_eq!(
            entry
                .country()
                .map(|code| code.as_str().to_owned())
                .as_deref(),
            country,
            "{isin}"
        );
        let stated = listings(row);
        assert_eq!(entry.listings().len(), stated.len(), "{isin}");
        for listing in &stated {
            let market = cell(listing, "miccode").map(mic);
            let held = entry
                .listing(market.as_ref())
                .unwrap_or_else(|| panic!("{isin}"));
            assert_eq!(held.ticker(), cell(listing, "ticker"), "{isin}");
            assert_eq!(
                held.currency().map(|code| code.as_str()),
                cell(listing, "currency"),
                "{isin}"
            );
        }
    }
    assert_eq!(
        seeded.iter().filter(|row| row.fisn().is_some()).count(),
        rows.iter().filter(|row| row.contains_key("fisn")).count()
    );
    assert_eq!(
        seeded.iter().filter(|row| row.fisn().is_some()).count(),
        181,
        "one short name per instrument: HSBC's once, where its two listing rows counted it twice"
    );
}

/// An instrument listed on two markets is one element with two listings:
/// HSBC on the London Stock Exchange and in Hong Kong, each listing its own
/// ticker and currency, the instrument's facts once.
#[test]
fn hsbc_is_one_instrument_listed_in_london_and_hong_kong() {
    crate::install::installed();
    const HSBC: &str = "GB0005405286";
    let seeded = Instruments::seeded();
    let hsbc = seeded.get(HSBC).expect("seeded");
    let [hong_kong, london] = hsbc.listings() else {
        panic!("two listings")
    };
    fn facts(listing: &Listing) -> (Option<&str>, Option<&str>, Option<&str>) {
        (
            listing.miccode().map(Mic::as_str),
            listing.ticker(),
            listing.currency().map(|code| code.as_str()),
        )
    }
    assert_eq!(facts(hong_kong), (Some("XHKG"), Some("0005"), Some("HKD")));
    assert_eq!(facts(london), (Some("XLON"), Some("HSBA"), Some("GBP")));
    assert_eq!(
        hsbc.cficode().map(|code| code.as_str().to_owned()),
        Some("ESVUFR".into())
    );
    assert_eq!(
        hsbc.fisn().map(|name| name.as_str().to_owned()),
        Some("HSBC HLDG/PAR VTG FPD 0.5".into())
    );
    assert_eq!(
        hsbc.get(&IdType::Sedol),
        Some("0540528"),
        "the embedded SEDOL, on the single-market rule's one holder"
    );
    for (ticker, market) in [("0005", "XHKG"), ("HSBA", "XLON")] {
        assert_eq!(
            seeded
                .get_by_ticker(ticker, None)
                .map(Element::get_crosscode),
            Some(HSBC),
            "{ticker} on {market}"
        );
        assert_eq!(hsbc.ticker(Some(&mic(market))), Some(ticker));
    }
    assert_eq!(hsbc.ticker(None), None, "two listings, no single ticker");
}

/// The seed is clean and bound to no store, and every seeded collection
/// answers the same instruments.
#[test]
fn the_seed_is_clean_and_unbound() {
    crate::install::installed();
    let seeded = Instruments::seeded();
    assert!(!seeded.is_dirty());
    assert!(seeded.holder().is_none());
    assert!(seeded.iter().eq(Instruments::seeded().iter()));
}

/// Apple by its ticker on its market: its ISIN, its listing's currency, its
/// country, its classification, its short name and the CUSIP its ISIN
/// embeds, derived as for any statement.
#[test]
fn apple_is_found_by_its_ticker_on_its_market() {
    crate::install::installed();
    let seeded = Instruments::seeded();
    let apple = seeded
        .get_by_ticker("AAPL", Some(&mic("XNAS")))
        .expect("listed");
    assert_eq!(apple.isin(), Some("US0378331005"));
    let nasdaq = apple.listing(Some(&mic("XNAS"))).expect("the listing");
    assert_eq!(nasdaq.currency().map(|code| code.as_str()), Some("USD"));
    assert_eq!(
        apple
            .country()
            .map(|code| code.as_str().to_owned())
            .as_deref(),
        Some("US")
    );
    assert_eq!(
        apple.cficode().map(|code| code.as_str().to_owned()),
        Some("ESVUFR".into())
    );
    assert_eq!(
        apple.fisn().map(|name| name.as_str().to_owned()),
        Some("APPLE INC/SH SH".into())
    );
    assert_eq!(apple.get(&IdType::Cusip), Some("037833100"));
    assert!(seeded.get_by_ticker("AAPL", Some(&mic("XLON"))).is_none());
    // One ticker on two instruments names each on its own market and
    // neither where no market is stated.
    assert_eq!(
        seeded
            .get_by_ticker("INFY", Some(&mic("XNSE")))
            .and_then(Instrument::isin),
        Some("INE009A01021")
    );
    assert!(seeded.get_by_ticker("INFY", None).is_none(), "ambiguous");
}

/// An index trades on no market and states no short name - its one listing
/// the unlisted one, holding its ticker and currency; an index of a
/// supranational prefix states the country it measures where one is
/// listed, and none where it measures several.
#[test]
fn an_index_states_no_market_and_no_short_name() {
    crate::install::installed();
    let seeded = Instruments::seeded();
    let spx = seeded.get("US78378X1072").expect("the S&P 500");
    assert_eq!(spx.ticker(None), Some("SPX"));
    let [unlisted] = spx.listings() else {
        panic!("one unlisted listing")
    };
    assert_eq!(unlisted.miccode(), None);
    assert_eq!(unlisted.currency().map(|code| code.as_str()), Some("USD"));
    assert_eq!(spx.fisn(), None);
    assert_eq!(
        spx.cficode().map(|code| code.as_str().to_owned()),
        Some("TIEXXX".into())
    );
    let asx = seeded.get("XC0006013624").expect("the ASX 200");
    assert_eq!(asx.countrycode().map(|code| code.as_str()), Some("AU"));
    for supranational in ["EU0009658145", "EU0009658202"] {
        let index = seeded.get(supranational).expect("an index");
        assert_eq!(index.listings()[0].miccode(), None, "{supranational}");
        assert_eq!(index.country(), None, "{supranational}: no one country");
    }
    let indices: Vec<&Instrument> = seeded
        .iter()
        .filter(|row| {
            row.listings()
                .iter()
                .all(|listing| listing.miccode().is_none())
        })
        .collect();
    assert_eq!(indices.len(), 17);
    for index in &indices {
        let code = index.cficode().expect("classified");
        assert!(
            yggdryl::Cfi::is_detailed(code.as_str()),
            "{}",
            index.get_crosscode()
        );
        assert_eq!(code.as_str(), "TIEXXX", "{}", index.get_crosscode());
        assert_eq!(index.fisn(), None, "{}", index.get_crosscode());
    }
    assert!(!yggdryl::Cfi::is_detailed("MRIXXX"));
}

/// The seed states the origin currency of the Irish USD share classes
/// alone, each listed in another currency, and derives none from a prefix.
#[test]
fn the_seed_states_the_origin_currency_of_the_irish_usd_share_classes_alone() {
    crate::install::installed();
    let seeded = Instruments::seeded();
    let origin = |isin: &str| {
        seeded
            .get(isin)
            .unwrap_or_else(|| panic!("{isin}: seeded"))
            .origccy()
            .map(|code| code.as_str().to_owned())
    };
    for (isin, currency) in [
        ("IE00B3RBWM25", "GBP"),
        ("IE00B3XXRP09", "GBP"),
        ("IE00B4L5Y983", "EUR"),
        ("IE00B5BMR087", "USD"),
        ("IE00B6R52259", "EUR"),
    ] {
        assert_eq!(
            origin(isin).as_deref(),
            Some("USD"),
            "{isin}: a USD share class"
        );
        let listing = &seeded.get(isin).unwrap().listings()[0];
        assert_eq!(
            listing.currency().map(|code| code.as_str()),
            Some(currency),
            "{isin}: the listing's own"
        );
    }
    assert_eq!(
        origin("KYG875721634"),
        None,
        "Tencent: no KYD from the prefix"
    );
    assert_eq!(
        origin("IE00B4BNMY34"),
        None,
        "Accenture: no EUR from the prefix"
    );
    assert_eq!(
        seeded.iter().filter(|row| row.origccy().is_some()).count(),
        5,
        "the five Irish USD share classes"
    );
}

/// A seed instrument derives the national number its ISIN embeds, as any
/// instrument the collection folds does; a listing code lands on its one
/// listing.
#[test]
fn a_seed_instrument_holds_the_national_number_its_isin_embeds() {
    crate::install::installed();
    let seeded = Instruments::seeded();
    for (isin, kind, code) in [
        ("GB0002374006", IdType::Sedol, "0237400"),
        ("CH0012214059", IdType::Valor, "1221405"),
        ("DE0007164600", IdType::Wkn, "716460"),
    ] {
        assert_eq!(
            seeded.get(isin).and_then(|row| row.get(&kind)),
            Some(code),
            "{isin}"
        );
    }
    let diageo = seeded.get("GB0002374006").unwrap();
    assert_eq!(
        diageo.listings()[0].get(&IdType::Sedol),
        Some("0237400"),
        "a SEDOL is a listing code"
    );
    assert_eq!(
        seeded.get("FR0000120578").map(|row| row
            .securityids()
            .iter()
            .map(|id| id.kind().clone())
            .collect::<Vec<_>>()),
        Some(vec![IdType::Cfi, IdType::Fisn, IdType::Isin]),
        "no scheme this crate checks: the key facts alone"
    );
}

#[cfg(feature = "internals")]
mod internal {
    use smol_str::SmolStr;
    use yggdryl_market::internals::instrument_seed::read_one;

    /// A seed object's key no typed field reads is a complementary fact of
    /// the instrument, its value text; a value of another shape is refused
    /// on the key.
    #[test]
    fn a_seed_objects_extra_key_is_a_metadata_entry() {
        crate::install::installed();
        let row = yggdryl::from_json_scalar(
            r#"{"isin": "US0378331005", "cficode": "ESVUFR", "countrycode": "US", "issuer": "Apple Inc.", "listings": [{"miccode": "XNAS", "ticker": "AAPL", "currency": "USD"}]}"#,
        )
        .unwrap();
        let apple = read_one(&row).unwrap();
        assert_eq!(
            apple.metadata().get("issuer").map(SmolStr::as_str),
            Some("Apple Inc.")
        );
        assert_eq!(apple.metadata().len(), 1);
        let numbered = yggdryl::from_json_scalar(
            r#"{"isin": "US0378331005", "cficode": "ESVUFR", "rank": 7}"#,
        )
        .unwrap();
        let error = read_one(&numbered).unwrap_err().to_string();
        assert!(error.contains("$.rank"), "{error}");
    }
}
