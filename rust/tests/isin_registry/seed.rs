//! `rust/src/isin_registry/seed.rs`: the seed every process default starts
//! from - `config/isin/instruments.json`, embedded at build time through the
//! crate's copy `rust/src/isin_registry/seed.json` - read as ordinary
//! statements, so what the embedded copy holds is pinned here and the data
//! cannot rot silently.

use std::collections::BTreeMap;

use smol_str::SmolStr;
use yggdryl::{CodeValue, IdType, IsinEntry, IsinRegistry, Mic, Scalar};

/// The text the crate embeds: its copy of the seed, which its package
/// carries.
const EMBEDDED: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/isin_registry/seed.json"
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

fn mic(text: &str) -> Mic {
    Mic::new(text).unwrap()
}

/// The crate's copy is `config/isin/instruments.json` byte for byte, where
/// the repository holds that file - a checkout, never the published crate,
/// which carries the copy alone: `python scripts/check_isin_seed.py --sync`
/// writes it.
#[test]
fn the_embedded_seed_is_the_config_file_byte_for_byte() {
    let config = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("isin")
        .join("instruments.json");
    if !config.is_file() {
        println!("SKIPPED: no {} beside the crate", config.display());
        return;
    }
    let held = std::fs::read(&config).expect("the seed file");
    assert!(
        held == EMBEDDED.as_bytes(),
        "rust/src/isin_registry/seed.json differs from config/isin/instruments.json:          run python scripts/check_isin_seed.py --sync"
    );
}

/// Every row of the document reaches the seed once, in ISIN order, keyed by
/// a real ISIN, with every value it states: none is refused or lands null.
#[test]
fn the_seed_holds_every_row_of_the_document_once_in_isin_order() {
    let rows = document();
    let seeded = IsinRegistry::seeded();
    assert_eq!(rows.len(), 208);
    assert_eq!(seeded.len(), rows.len());
    let stated: Vec<&str> = rows.iter().map(|row| cell(row, "isin").unwrap()).collect();
    let held: Vec<&str> = seeded.iter().map(|row| row.isin().as_str()).collect();
    assert_eq!(held, stated, "unique and sorted by ISIN");
    assert_eq!(held.first(), Some(&"AU000000BHP4"));
    assert_eq!(held.last(), Some(&"XC0006013624"));
    assert!(seeded.iter().all(|row| row.isin().is_real()));
    for row in &rows {
        let isin = cell(row, "isin").unwrap();
        let entry = seeded.get(isin).unwrap();
        assert_eq!(entry.ticker(), cell(row, "ticker"), "{isin}");
        assert_eq!(
            entry.miccode().map(Mic::as_str),
            cell(row, "miccode"),
            "{isin}"
        );
        assert_eq!(
            entry.currency().map(|code| code.as_str()),
            cell(row, "currency"),
            "{isin}"
        );
        assert_eq!(
            entry.cficode().map(|code| code.as_str()),
            cell(row, "cficode"),
            "{isin}"
        );
        assert_eq!(
            entry.fisn().map(yggdryl::Fisn::as_str),
            cell(row, "fisn"),
            "{isin}"
        );
        // A listed country is the row's country; the ISIN's own prefix is
        // held as none beside it, and a reserved code (`EZ`) is no country.
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
    }
    assert_eq!(
        seeded.iter().filter(|row| row.fisn().is_some()).count(),
        rows.iter().filter(|row| row.contains_key("fisn")).count()
    );
    assert_eq!(
        seeded.iter().filter(|row| row.fisn().is_some()).count(),
        181
    );
}

/// The seed is clean and bound to no store, and every seeded registry
/// answers the same rows.
#[test]
fn the_seed_is_clean_and_unbound() {
    let seeded = IsinRegistry::seeded();
    assert!(!seeded.is_dirty());
    assert!(seeded.holder().is_none());
    assert!(seeded.iter().eq(IsinRegistry::seeded().iter()));
}

/// Apple by its ticker on its market: its ISIN, its trading currency, its
/// country, its classification, its short name and the CUSIP its ISIN
/// embeds, derived as for any statement.
#[test]
fn apple_is_found_by_its_ticker_on_its_market() {
    let seeded = IsinRegistry::seeded();
    let apple = seeded
        .get_by_ticker("AAPL", Some(&mic("XNAS")))
        .expect("listed");
    assert_eq!(apple.isin().as_str(), "US0378331005");
    assert_eq!(apple.miccode().map(Mic::as_str), Some("XNAS"));
    assert_eq!(apple.currency().map(|code| code.as_str()), Some("USD"));
    assert_eq!(
        apple
            .country()
            .map(|code| code.as_str().to_owned())
            .as_deref(),
        Some("US")
    );
    assert_eq!(apple.cficode().map(|code| code.as_str()), Some("ESVUFR"));
    assert_eq!(
        apple.fisn().map(yggdryl::Fisn::as_str),
        Some("APPLE INC/SH SH")
    );
    assert_eq!(apple.get(&IdType::Cusip), Some("037833100"));
    assert!(seeded.get_by_ticker("AAPL", Some(&mic("XLON"))).is_none());
    // One ticker on two markets names each listing on its own and neither
    // where no market is stated.
    assert_eq!(
        seeded
            .get_by_ticker("INFY", Some(&mic("XNSE")))
            .map(|row| row.isin().as_str()),
        Some("INE009A01021")
    );
    assert!(seeded.get_by_ticker("INFY", None).is_none(), "ambiguous");
}

/// An index trades on no market and states no short name; an index of a
/// supranational prefix states the country it measures where one is
/// listed, and none where it measures several.
#[test]
fn an_index_row_states_no_market_and_no_short_name() {
    let seeded = IsinRegistry::seeded();
    let spx = seeded.get("US78378X1072").expect("the S&P 500");
    assert_eq!(spx.ticker(), Some("SPX"));
    assert_eq!(spx.miccode(), None);
    assert_eq!(spx.fisn(), None);
    assert_eq!(spx.cficode().map(|code| code.as_str()), Some("TIEXXX"));
    assert_eq!(spx.currency().map(|code| code.as_str()), Some("USD"));
    let asx = seeded.get("XC0006013624").expect("the S&P/ASX 200");
    assert_eq!(asx.countrycode().map(|code| code.as_str()), Some("AU"));
    for supranational in ["EU0009658145", "EU0009658202"] {
        let index = seeded.get(supranational).expect("a STOXX index");
        assert_eq!(index.miccode(), None, "{supranational}");
        assert_eq!(index.country(), None, "{supranational}: no one country");
        assert_eq!(
            index.currency().map(|code| code.as_str()),
            Some("EUR"),
            "{supranational}"
        );
    }
    // Every index row's classification is stored, detailed under the
    // ISO 10962:2021 table the crate reads - referential, indices,
    // equities - never 2015's `MRIXXX`, which that table reads as nothing.
    let indices: Vec<&IsinEntry> = seeded
        .iter()
        .filter(|row| row.miccode().is_none())
        .collect();
    assert_eq!(indices.len(), 17);
    for index in indices {
        let code = index
            .cficode()
            .unwrap_or_else(|| panic!("{}: a stored classification", index.isin()));
        assert!(yggdryl::Cfi::is_detailed(code.as_str()), "{}", index.isin());
        assert_eq!(code.as_str(), "TIEXXX", "{}", index.isin());
    }
    assert!(!yggdryl::Cfi::is_detailed("MRIXXX"));
}

/// A seed row derives the national number its ISIN embeds, as any row the
/// registry folds does.
#[test]
fn a_seed_row_holds_the_national_number_its_isin_embeds() {
    let seeded = IsinRegistry::seeded();
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
    assert!(
        seeded
            .get("FR0000120578")
            .is_some_and(|row: &IsinEntry| row.iter().next().is_none()),
        "no scheme this crate checks"
    );
}
