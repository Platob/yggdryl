//! `rust/market/src/isin_registry.rs`: one row per ISIN and market of every fact
//! an instrument is known by - learned from statements, filled into the
//! ones that leave it unsaid, a valid value filling and replacing whatever
//! the time, the instrument's facts on every listing and a listing's on its
//! own row, a ticker or a lookup code leading back to its ISIN, the
//! `resolve` waterfall over them and the economic match - read from and
//! written to a holder through the Arrow record surface.

use std::sync::Arc;

use arrow_array::{Int32Array, Int64Array, RecordBatch, StringArray, UInt16Array};
use arrow_schema::{DataType as ArrowType, Field as ArrowField, Schema};
use smol_str::SmolStr;
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{Ccy, Cfi, Country, Fisn, Forex, IOBase, IOMedia, IOMode, Isin, Mic};
use yggdryl_market::graph::{Market, OrderEvent};
use yggdryl_market::{
    Eusipa, IdKey, IdType, Identifier, IsinEntry, IsinRegistry, MatchTier, Resolution, Unmatched,
};

const HOLCIM: &str = "CH0012214059";
const APPLE: &str = "US0378331005";
const NOVARTIS: &str = "CH0012005267";
const DIAGEO: &str = "GB0002374006";
const SAP: &str = "DE0007164600";
const HSBC: &str = "GB0005405286";

fn isin(text: &str) -> Isin {
    Isin::new(text).unwrap()
}

/// A real ISIN under `prefix` numbered zero.
fn numbered(prefix: &str) -> String {
    let body = format!("{prefix}000000000");
    format!("{body}{}", Isin::closing_digit(&body).unwrap())
}

fn id(kind: IdType, value: &str) -> Identifier {
    Identifier::new(IdKey::base(kind), value).unwrap()
}

/// An order at `unix` stating each of `codes`.
fn order(unix: i64, codes: &[(IdType, &str)]) -> OrderEvent {
    let mut event = OrderEvent::at(unix);
    for (kind, value) in codes {
        event
            .insert_securityid(id(kind.clone(), value))
            .expect("a plain holder takes every security identifier");
    }
    event
}

/// The row of `text` at `unix`, stating each of `codes`.
fn entry(text: &str, unix: Option<i64>, codes: &[(IdType, &str)]) -> IsinEntry {
    codes.iter().fold(
        IsinEntry::new(isin(text)).with_updunix(unix),
        |entry, (kind, value)| entry.try_with_code(kind.clone(), value).unwrap(),
    )
}

fn mic(text: &str) -> Option<Mic> {
    Some(Mic::new(text).unwrap())
}

fn cfi(text: &str) -> Option<Cfi> {
    Some(Cfi::new(text).unwrap())
}

fn ccy(text: &str) -> Option<Ccy> {
    Some(Ccy::new(text).unwrap())
}

fn fisn(text: &str) -> Option<Fisn> {
    Some(Fisn::new(text).unwrap())
}

#[test]
fn a_statement_is_learned_by_its_isin_and_filled_into_one_naming_it() {
    crate::install::installed();
    let mut stated = order(
        10,
        &[
            (IdType::Isin, HOLCIM),
            (IdType::Ric, "HOLN.S"),
            (IdType::Bloomberg, "HOLN SW Equity"),
            (IdType::Common, "C-1"),
        ],
    );
    stated.set_cficode(cfi("ESVUFR"), true);
    stated.set_miccode(mic("XSWX"), true);
    stated.set_ticker(Some(SmolStr::new("HOLN")), true);
    stated.set_currency(Ccy::new("CHF").unwrap(), true);
    let mut registry = IsinRegistry::new();
    assert!(!registry.is_dirty());
    assert!(registry.learn(&stated));
    assert!(registry.is_dirty());
    assert!(!registry.learn(&stated), "nothing new");
    let row = registry.get(HOLCIM).expect("a row");
    assert_eq!(row.updunix(), Some(10));
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(row.get(&IdType::Bloomberg), Some("HOLN SW Equity"));
    assert_eq!(row.cficode().map(|code| code.as_str()), Some("ESVUFR"));
    assert_eq!(row.miccode().map(|code| code.as_str()), Some("XSWX"));
    assert_eq!(row.ticker(), Some("HOLN"));
    assert_eq!(row.currency().map(|code| code.as_str()), Some("CHF"));
    assert_eq!(row.countrycode(), None, "no statement named a country");
    assert_eq!(
        row.country(),
        Some(Country::new("CH").unwrap()),
        "the prefix"
    );
    assert_eq!(row.forexcode(), None);

    // The ISIN alone fills the rest, each as a derivation.
    let mut named = order(20, &[(IdType::Isin, HOLCIM)]);
    assert!(registry.fill(&mut named));
    let ids = named.get_securityids();
    assert_eq!(ids.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(ids.get(&IdType::Common), Some("C-1"));
    assert!(ids.is_derived(&IdType::Ric));
    assert!(!ids.is_derived(&IdType::Isin));
    assert_eq!(named.get_ticker(), Some("HOLN"));
    assert_eq!(
        named.get_cficode().map(|code| code.as_str()),
        Some("ESVUFR")
    );
    assert!(
        named.get_currency().is_none(),
        "the currency fills only where the markets are stated and equal"
    );
    assert!(!registry.fill(&mut named), "nothing left to fill");
    // What a fill derived is never learned back: learning the element moves
    // the instant the instrument was met, and no fact.
    assert!(registry.learn(&named));
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!((row.updunix(), row.lastunix()), (Some(10), Some(20)));
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"));
    assert!(!registry.learn(&named), "met again at the same instant");

    // On the listing's market, with the row's ticker, the currency fills too.
    let mut listed = order(20, &[(IdType::Isin, HOLCIM)]);
    listed.set_miccode(mic("XSWX"), true);
    assert!(registry.fill(&mut listed));
    assert_eq!(listed.get_ticker(), Some("HOLN"));
    assert_eq!(listed.get_currency().as_str(), "CHF");

    // A statement of its own stands; a refined CFI code refines it.
    let mut own = order(30, &[(IdType::Isin, HOLCIM), (IdType::Common, "C-9")]);
    own.set_cficode(cfi("ESXUFR"), true);
    assert!(registry.fill(&mut own));
    assert_eq!(own.get_securityids().get(&IdType::Common), Some("C-9"));
    assert_eq!(own.get_cficode().map(|code| code.as_str()), Some("ESVUFR"));
}

/// A RIC is a listing code like a Bloomberg symbol, and one of the
/// lookup codes (`IsinRegistry::LOOKUP_CODES`) since local codes became
/// keys: with no ISIN, an element naming only the RIC resolves to the one
/// instrument holding it, its ISIN derived; a code two instruments hold
/// resolves to none. A learn stays keyed by a stated ISIN alone.
#[test]
fn a_ric_is_a_listing_code_and_with_no_isin_a_lookup_key() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    registry
        .merge(
            entry(
                HOLCIM,
                Some(10),
                &[(IdType::Ric, "HOLN.S"), (IdType::Common, "C-1")],
            )
            .with_miccode(mic("XSWX")),
        )
        .unwrap();
    // An element naming only the RIC resolves to the instrument holding it.
    let mut by_ric = order(20, &[(IdType::Ric, "HOLN.S")]);
    assert!(registry.fill(&mut by_ric));
    assert_eq!(by_ric.get_isincode(), Some(HOLCIM));
    assert!(by_ric.get_securityids().is_derived(&IdType::Isin));
    assert_eq!(by_ric.get_securityids().get(&IdType::Common), Some("C-1"));
    // A statement naming a RIC and no ISIN is learned by nothing.
    assert!(!registry.learn(&order(
        30,
        &[(IdType::Ric, "HOLN.S"), (IdType::Belgian, "B-1")]
    )));
    assert_eq!(registry.get(HOLCIM).unwrap().get(&IdType::Belgian), None);
    // Two instruments may hold one RIC, and a fill by its ISIN hands each
    // its own; by the RIC alone, neither.
    registry
        .merge(entry(APPLE, Some(10), &[(IdType::Ric, "HOLN.S")]))
        .unwrap();
    assert_eq!(
        registry.get(APPLE).unwrap().get(&IdType::Ric),
        Some("HOLN.S")
    );
    assert_eq!(
        registry.get(HOLCIM).unwrap().get(&IdType::Ric),
        Some("HOLN.S")
    );
    let mut apple = order(40, &[(IdType::Isin, APPLE)]);
    assert!(registry.fill(&mut apple));
    assert_eq!(apple.get_securityids().get(&IdType::Ric), Some("HOLN.S"));
    let mut ambiguous = order(40, &[(IdType::Ric, "HOLN.S")]);
    assert!(!registry.fill(&mut ambiguous), "two instruments hold it");
    assert_eq!(ambiguous.get_isincode(), None);
    assert_eq!(registry.get_by_code(&IdType::Ric, "HOLN.S", None), None);
    // On the row's market or none, the RIC fills; on another, it does not.
    let mut elsewhere = order(40, &[(IdType::Isin, HOLCIM)]);
    elsewhere.set_miccode(mic("XLON"), true);
    assert!(registry.fill(&mut elsewhere), "the instrument's code fills");
    assert_eq!(elsewhere.get_securityids().get(&IdType::Ric), None);
    assert_eq!(
        elsewhere.get_securityids().get(&IdType::Common),
        Some("C-1")
    );
}

/// A Bloomberg symbol is a lookup key as a RIC is; a statement without an
/// ISIN is still learned by nothing.
#[test]
fn a_bloomberg_symbol_keys_a_lookup_and_no_learn() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    registry
        .merge(entry(
            HOLCIM,
            Some(10),
            &[(IdType::Bloomberg, "HOLN SW Equity")],
        ))
        .unwrap();
    let mut symbol = order(20, &[(IdType::Bloomberg, "HOLN SW Equity")]);
    assert!(registry.fill(&mut symbol));
    assert_eq!(symbol.get_isincode(), Some(HOLCIM));
    assert!(
        !registry.learn(&order(
            20,
            &[
                (IdType::Bloomberg, "HOLN SW Equity"),
                (IdType::Common, "C-1")
            ]
        )),
        "a statement without an ISIN is learned by nothing"
    );
    assert_eq!(registry.get(HOLCIM).unwrap().get(&IdType::Common), None);
}

/// No clock gates a merge: a stated valid value fills a column the row
/// lacks and replaces one it holds that differs, an older or undated
/// statement included; the same value moves nothing; `updunix` is the
/// later of the two, a stamp.
#[test]
fn a_valid_statement_fills_and_replaces_whatever_the_time() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    assert!(
        registry
            .merge(entry(HOLCIM, Some(10), &[(IdType::Common, "A")]))
            .unwrap()
    );
    // Older: fills what the row lacks and replaces what differs; the stamp
    // never moves back.
    assert!(
        registry
            .merge(entry(
                HOLCIM,
                Some(5),
                &[(IdType::Common, "B"), (IdType::Belgian, "X")]
            ))
            .unwrap()
    );
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Common), Some("B"));
    assert_eq!(row.get(&IdType::Belgian), Some("X"));
    assert_eq!(row.updunix(), Some(10));
    // Newer: replaces, and the stamp follows.
    assert!(
        registry
            .merge(entry(HOLCIM, Some(20), &[(IdType::Common, "D")]))
            .unwrap()
    );
    assert_eq!(
        registry.get(HOLCIM).unwrap().get(&IdType::Common),
        Some("D")
    );
    assert_eq!(registry.get(HOLCIM).unwrap().updunix(), Some(20));
    // Undated: replaces too.
    assert!(
        registry
            .merge(entry(HOLCIM, None, &[(IdType::Common, "E")]))
            .unwrap()
    );
    assert_eq!(
        registry.get(HOLCIM).unwrap().get(&IdType::Common),
        Some("E")
    );
    assert_eq!(registry.get(HOLCIM).unwrap().updunix(), Some(20));
    // Nothing that differs moves nothing, and keeps the registry clean.
    registry.clear();
    registry
        .merge(entry(HOLCIM, Some(20), &[(IdType::Common, "E")]))
        .unwrap();
    let mut again = IsinRegistry::new();
    again
        .merge(entry(HOLCIM, Some(20), &[(IdType::Common, "E")]))
        .unwrap();
    assert!(
        !again
            .merge(entry(HOLCIM, Some(99), &[(IdType::Common, "E")]))
            .unwrap()
    );
    assert_eq!(again.get(HOLCIM).unwrap().updunix(), Some(20));
    // Removing and clearing move the registry; clearing nothing does not.
    let mut moved = IsinRegistry::new();
    assert!(!moved.is_dirty());
    moved.clear();
    assert!(
        !moved.is_dirty(),
        "clearing an empty registry moves nothing"
    );
    moved.merge(entry(APPLE, None, &[])).unwrap();
    assert!(moved.is_dirty());
    assert_eq!(moved.remove(APPLE).len(), 1);
    assert!(moved.remove(APPLE).is_empty());
    assert!(moved.is_dirty());
}

/// Only a real value moves a column: a typo under a checked code is
/// dropped from the statement, whatever its date, and a real value
/// replaces a real one; the key is a real ISIN.
#[test]
fn an_invalid_value_moves_nothing_and_a_real_one_replaces() {
    crate::install::installed();
    const REAL: &str = "037833100";
    const TYPO: &str = "037833101";
    const OTHER: &str = "594918104";
    let mut registry = IsinRegistry::new();
    assert!(
        registry
            .merge(entry(HOLCIM, Some(20), &[(IdType::Cusip, TYPO)]))
            .unwrap(),
        "the row is created, holding no code"
    );
    assert_eq!(registry.get(HOLCIM).unwrap().get(&IdType::Cusip), None);
    assert!(
        registry
            .merge(entry(HOLCIM, Some(10), &[(IdType::Cusip, REAL)]))
            .unwrap()
    );
    assert_eq!(
        registry.get(HOLCIM).unwrap().get(&IdType::Cusip),
        Some(REAL)
    );
    assert_eq!(registry.get(HOLCIM).unwrap().updunix(), Some(20));
    // A newer typo never replaces a real code.
    assert!(
        !registry
            .merge(entry(HOLCIM, Some(30), &[(IdType::Cusip, TYPO)]))
            .unwrap()
    );
    assert_eq!(
        registry.get(HOLCIM).unwrap().get(&IdType::Cusip),
        Some(REAL)
    );
    // Two real codes: the one stated replaces, whatever the time.
    assert!(
        registry
            .merge(entry(HOLCIM, Some(15), &[(IdType::Cusip, OTHER)]))
            .unwrap()
    );
    assert_eq!(
        registry.get(HOLCIM).unwrap().get(&IdType::Cusip),
        Some(OTHER)
    );
    assert_eq!(registry.get(HOLCIM).unwrap().updunix(), Some(20));

    // The key is a real number: a masked or mistyped one names no row.
    for unreal in ["XX0000000001", "CH0012214058", "ZZ0000000008"] {
        let refused = IsinRegistry::new()
            .merge(IsinEntry::new(isin(unreal)))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("$.isin"), "{unreal}: {refused}");
        assert!(refused.contains(unreal), "{unreal}: {refused}");
    }
    // A referential number keys a row: `XT` is an agency prefix.
    let referential = numbered("XT");
    assert!(
        IsinRegistry::new()
            .merge(IsinEntry::new(isin(&referential)))
            .unwrap()
    );
    // Learning keys only a real stated ISIN and collects only the
    // equivalents that are real: a masked number learns nothing and a typo
    // beside a real key is left out.
    let mut registry = IsinRegistry::new();
    let mut masked = order(
        1,
        &[(IdType::Isin, "XX0000000001"), (IdType::Common, "C-1")],
    );
    masked.set_ticker(Some(SmolStr::new("MASK")), true);
    assert!(!registry.learn(&masked));
    assert!(registry.is_empty());
    let typed = order(
        2,
        &[
            (IdType::Isin, HOLCIM),
            (IdType::Cusip, TYPO),
            (IdType::Common, "C-1"),
        ],
    );
    assert!(registry.learn(&typed));
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Cusip), None);
    assert_eq!(row.get(&IdType::Common), Some("C-1"));
}

/// A ticker leads back to its ISIN through an inverse index, gated by the
/// market: an element stating only the ticker takes the row's ISIN where
/// the markets agree or either is unstated and exactly one row lists the
/// ticker, and a masked ISIN it states is replaced by the row's real one.
#[test]
fn a_registry_fills_the_isin_a_ticker_names_on_the_same_market() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    let mut listed = order(10, &[(IdType::Isin, HOLCIM), (IdType::Ric, "HOLN.S")]);
    listed.set_ticker(Some(SmolStr::new("HOLN")), true);
    listed.set_miccode(mic("XSWX"), true);
    assert!(registry.learn(&listed));
    assert_eq!(
        registry
            .get_by_ticker("HOLN", None)
            .map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );
    assert_eq!(
        registry
            .get_by_ticker("HOLN", mic("XSWX").as_ref())
            .map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );
    assert!(
        registry
            .get_by_ticker("HOLN", mic("XLON").as_ref())
            .is_none()
    );
    assert!(registry.get_by_ticker("ABBN", None).is_none());
    // A ticker is looked up trimmed, as it is learned.
    assert_eq!(
        registry
            .get_by_ticker(" HOLN ", None)
            .map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );
    let mut padded = OrderEvent::at(20);
    padded.set_ticker(Some(SmolStr::new(" HOLN ")), true);
    padded.set_miccode(mic("XSWX"), true);
    assert!(registry.fill(&mut padded));
    assert_eq!(padded.get_securityids().get(&IdType::Isin), Some(HOLCIM));

    // A ticker-only statement, no market: the ISIN derives, the rest with it.
    let mut unstated = OrderEvent::at(20);
    unstated.set_ticker(Some(SmolStr::new("HOLN")), true);
    assert!(registry.fill(&mut unstated));
    assert_eq!(unstated.get_isincode(), Some(HOLCIM));
    assert!(unstated.get_securityids().is_derived(&IdType::Isin));
    assert_eq!(unstated.get_securityids().get(&IdType::Ric), Some("HOLN.S"));
    // On the row's market: the same; on another market: nothing.
    let mut same = OrderEvent::at(20);
    same.set_ticker(Some(SmolStr::new("HOLN")), true);
    same.set_miccode(mic("XSWX"), true);
    assert!(registry.fill(&mut same));
    assert_eq!(same.get_isincode(), Some(HOLCIM));
    let mut other = OrderEvent::at(20);
    other.set_ticker(Some(SmolStr::new("HOLN")), true);
    other.set_miccode(mic("XLON"), true);
    assert!(!registry.fill(&mut other));
    assert_eq!(other.get_isincode(), None);
    // A masked ISIN beside the ticker is replaced by the row's real one.
    let mut masked = order(20, &[(IdType::Isin, "XX0000000001")]);
    masked.set_ticker(Some(SmolStr::new("HOLN")), true);
    assert!(registry.fill(&mut masked));
    assert_eq!(masked.get_isincode(), Some(HOLCIM));
    // A real ISIN stated beside a ticker another row lists stands, and a
    // real ISIN no row holds ends the fill: the ticker is not asked.
    let mut stated = order(20, &[(IdType::Isin, APPLE)]);
    stated.set_ticker(Some(SmolStr::new("HOLN")), true);
    assert!(!registry.fill(&mut stated));
    assert_eq!(stated.get_isincode(), Some(APPLE));
    assert_eq!(stated.get_securityids().get(&IdType::Ric), None);

    // Two rows listing one ticker on two markets: a statement naming a
    // market resolves to that listing, one naming none resolves to neither.
    let mut other_listing = order(30, &[(IdType::Isin, NOVARTIS)]);
    other_listing.set_ticker(Some(SmolStr::new("HOLN")), true);
    other_listing.set_miccode(mic("XLON"), true);
    assert!(registry.learn(&other_listing));
    assert_eq!(
        registry
            .get_by_ticker("HOLN", mic("XLON").as_ref())
            .map(|row| row.isin().as_str()),
        Some(NOVARTIS)
    );
    assert_eq!(
        registry
            .get_by_ticker("HOLN", mic("XSWX").as_ref())
            .map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );
    assert!(registry.get_by_ticker("HOLN", None).is_none(), "ambiguous");
    let mut ambiguous = OrderEvent::at(40);
    ambiguous.set_ticker(Some(SmolStr::new("HOLN")), true);
    assert!(!registry.fill(&mut ambiguous));
    // The index follows a row's ticker: a listing fact moves it, and a
    // removed row leaves it.
    let mut renamed = order(50, &[(IdType::Isin, NOVARTIS)]);
    renamed.set_ticker(Some(SmolStr::new("NOVN")), true);
    renamed.set_miccode(mic("XLON"), true);
    assert!(registry.learn(&renamed));
    assert_eq!(
        registry
            .get_by_ticker("NOVN", None)
            .map(|row| row.isin().as_str()),
        Some(NOVARTIS)
    );
    assert_eq!(
        registry
            .get_by_ticker("HOLN", None)
            .map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );
    assert_eq!(registry.remove(HOLCIM).len(), 1);
    assert!(registry.get_by_ticker("HOLN", None).is_none());
    registry.clear();
    assert!(registry.get_by_ticker("NOVN", None).is_none());
}

#[test]
fn a_cfi_code_refines_and_a_conflict_replaces_whatever_the_time() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    let classified = |unix: i64, code: &str| {
        IsinEntry::new(isin(HOLCIM))
            .with_updunix(Some(unix))
            .with_cficode(cfi(code))
    };
    registry.merge(classified(10, "ESXUFR")).unwrap();
    assert!(registry.merge(classified(5, "ESVXXX")).unwrap(), "refined");
    let held = |registry: &IsinRegistry| {
        registry
            .get(HOLCIM)
            .unwrap()
            .cficode()
            .unwrap()
            .as_str()
            .to_owned()
    };
    assert_eq!(held(&registry), "ESVUFR");
    assert!(
        !registry.merge(classified(20, "ESVXXX")).unwrap(),
        "a coarser compatible code moves nothing"
    );
    assert!(
        registry.merge(classified(5, "ESNUFR")).unwrap(),
        "an older conflict replaces too"
    );
    assert_eq!(held(&registry), "ESNUFR");
    // A coarse code is stored as none.
    assert!(
        IsinEntry::new(isin(HOLCIM))
            .with_cficode(cfi("ESXXXX"))
            .cficode()
            .is_none()
    );
}

/// A statement naming another market is another listing of the instrument,
/// a currency alone among its facts too, created with the instrument's
/// facts and holding the listing facts it states; each listing keeps its
/// own, whatever the time, and the instrument's facts fold into every one.
#[test]
fn a_statement_on_another_market_is_another_listing_of_the_instrument() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    let listing = |unix: i64, market: &str, codes: &[(IdType, &str)]| {
        entry(HOLCIM, Some(unix), codes).with_miccode(mic(market))
    };
    registry
        .merge(
            listing(
                10,
                "XSWX",
                &[(IdType::Ric, "HOLN.S"), (IdType::Common, "C")],
            )
            .with_ticker(Some(SmolStr::new("HOLN")))
            .with_currency(ccy("CHF")),
        )
        .unwrap();
    // A currency alone on another market is a listing there.
    assert!(
        registry
            .merge(listing(20, "XLON", &[]).with_currency(ccy("USD")))
            .unwrap()
    );
    assert_eq!((registry.len(), registry.rows()), (1, 2));
    let london = registry.get_listing(HOLCIM, &mic("XLON").unwrap()).unwrap();
    assert_eq!(london.currency().map(Ccy::as_str), Some("USD"));
    assert_eq!(london.ticker(), None, "a listing's own facts");
    assert_eq!(london.get(&IdType::Ric), None);
    assert_eq!(london.get(&IdType::Common), Some("C"), "the instrument's");
    assert_eq!(
        registry.get(HOLCIM),
        Some(london),
        "the first listing, in MIC order"
    );
    let swiss = registry.get_listing(HOLCIM, &mic("XSWX").unwrap()).unwrap();
    assert_eq!(swiss.ticker(), Some("HOLN"));
    assert_eq!(swiss.currency().map(Ccy::as_str), Some("CHF"));
    assert_eq!(swiss.get(&IdType::Ric), Some("HOLN.S"));
    // A listing fact there - older, even - lands on that listing alone.
    assert!(
        registry
            .merge(listing(5, "XLON", &[(IdType::Ric, "HOLN.L")]))
            .unwrap()
    );
    let rics: Vec<_> = registry
        .listings(HOLCIM)
        .iter()
        .map(|row| (row.miccode().map(Mic::as_str), row.get(&IdType::Ric)))
        .collect();
    assert_eq!(
        rics,
        [
            (Some("XLON"), Some("HOLN.L")),
            (Some("XSWX"), Some("HOLN.S"))
        ]
    );
    // The instrument's facts fold into every listing, and the stamp is the
    // instrument's.
    assert!(
        registry
            .merge(entry(HOLCIM, Some(40), &[(IdType::Common, "C-2")]))
            .unwrap()
    );
    for row in registry.listings(HOLCIM) {
        assert_eq!(row.get(&IdType::Common), Some("C-2"), "{:?}", row.miccode());
        assert_eq!(row.updunix(), Some(40), "{:?}", row.miccode());
    }
    assert!(
        registry.get_by_ticker("HOLN", None).is_some(),
        "the one listing stating the ticker"
    );

    // A fill never carries one market's listing onto another's message.
    let mut paris = order(50, &[(IdType::Isin, HOLCIM)]);
    paris.set_miccode(mic("XPAR"), true);
    assert!(registry.fill(&mut paris));
    assert_eq!(paris.get_securityids().get(&IdType::Ric), None);
    assert_eq!(paris.get_ticker(), None);
    assert!(paris.get_currency().is_none());
    assert_eq!(
        paris.get_securityids().get(&IdType::Common),
        Some("C-2"),
        "the instrument's own columns fill everywhere"
    );
    let mut london = order(50, &[(IdType::Isin, HOLCIM)]);
    london.set_miccode(mic("XLON"), true);
    assert!(registry.fill(&mut london));
    assert_eq!(london.get_securityids().get(&IdType::Ric), Some("HOLN.L"));
    assert_eq!(london.get_currency().as_str(), "USD");
}

/// The country of issue, the currency pair and the trading currency are
/// learned where stated and filled where unsaid: a country only where ISO
/// 3166 lists it, else the ISIN's prefix answers it; a pair only as a
/// stated `forex` identifier, filled derived; a currency only where the
/// element holds no pair - `Currency(15)` on an FX trade is the dealt
/// currency - and filled only on the same stated market with the row's
/// ticker.
#[test]
fn the_country_the_pair_and_the_currency_are_learned_and_filled() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    // A stated country stands over the prefix; an unlisted one is none.
    registry
        .merge(
            IsinEntry::new(isin(HOLCIM))
                .with_countrycode(Some(Country::new("LI").unwrap()))
                .with_currency(ccy("XXX")),
        )
        .unwrap();
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.countrycode().map(|code| code.as_str()), Some("LI"));
    assert_eq!(
        row.country().map(|code| code.as_str().to_owned()),
        Some("LI".into())
    );
    assert_eq!(
        row.currency(),
        None,
        "XXX states no currency, and a row of no market takes no default"
    );
    assert!(
        IsinEntry::new(isin(HOLCIM))
            .with_countrycode(Some(Country::new("XX").unwrap()))
            .countrycode()
            .is_none()
    );
    // The ISIN's own prefix, stated, takes the held country back and is
    // held beside the key by no row.
    let own = || IsinEntry::new(isin(HOLCIM)).with_countrycode(Some(Country::new("CH").unwrap()));
    assert!(registry.merge(own()).unwrap(), "a stated country replaces");
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.countrycode(), None);
    assert_eq!(
        row.country().map(|code| code.as_str().to_owned()),
        Some("CH".into())
    );
    assert!(
        !registry.merge(own()).unwrap(),
        "restated, it moves nothing"
    );
    let mut fresh = IsinRegistry::new();
    fresh
        .merge(IsinEntry::new(isin(APPLE)).with_countrycode(Some(Country::new("US").unwrap())))
        .unwrap();
    assert_eq!(fresh.get(APPLE).unwrap().countrycode(), None);
    assert_eq!(
        fresh
            .get(APPLE)
            .unwrap()
            .country()
            .map(|code| code.as_str().to_owned()),
        Some("US".into())
    );
    // An agency prefix names no country.
    let referential = numbered("XT");
    registry.merge(IsinEntry::new(isin(&referential))).unwrap();
    assert_eq!(registry.get(&referential).unwrap().country(), None);

    // A pair and the dealt currency, learned off an FX trade.
    let fx = numbered("EZ");
    let mut traded = order(
        10,
        &[(IdType::Isin, fx.as_str()), (IdType::Forex, "eurusd")],
    );
    traded.set_currency(Ccy::new("EUR").unwrap(), true);
    traded.set_miccode(mic("XOFF"), true);
    assert!(registry.learn(&traded));
    let row = registry.get(&fx).unwrap();
    assert_eq!(row.forexcode().map(|pair| pair.as_str()), Some("EUR/USD"));
    assert_eq!(row.currency(), None, "the dealt currency is no listing's");
    assert_eq!(row.miccode().map(|code| code.as_str()), Some("XOFF"));
    // A later trade of the number takes the pair, derived.
    let mut later = order(20, &[(IdType::Isin, fx.as_str())]);
    assert!(registry.fill(&mut later));
    assert_eq!(later.get_securityids().get(&IdType::Forex), Some("EUR/USD"));
    assert!(later.get_securityids().is_derived(&IdType::Forex));
    // A derived pair is never learned back: the learn moves the instant the
    // number was met, and no fact.
    assert!(registry.learn(&later));
    let row = registry.get(&fx).unwrap();
    assert_eq!((row.updunix(), row.lastunix()), (Some(10), Some(20)));
    assert_eq!(row.forexcode().map(|pair| pair.as_str()), Some("EUR/USD"));
    // A pair stated otherwise replaces.
    let other = order(
        30,
        &[(IdType::Isin, fx.as_str()), (IdType::Forex, "EUR/CHF")],
    );
    assert!(registry.learn(&other));
    assert_eq!(
        registry
            .get(&fx)
            .unwrap()
            .forexcode()
            .map(|pair| pair.as_str()),
        Some("EUR/CHF")
    );

    // The trading currency, learned with the listing and filled on it.
    let mut listed = order(10, &[(IdType::Isin, HOLCIM)]);
    listed.set_ticker(Some(SmolStr::new("HOLN")), true);
    listed.set_miccode(mic("XSWX"), true);
    listed.set_currency(Ccy::new("CHF").unwrap(), true);
    assert!(registry.learn(&listed));
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.currency().map(|code| code.as_str()), Some("CHF"));
    // No market stated: the ticker fills, the currency does not.
    let mut unmarked = order(20, &[(IdType::Isin, HOLCIM)]);
    assert!(registry.fill(&mut unmarked));
    assert_eq!(unmarked.get_ticker(), Some("HOLN"));
    assert!(unmarked.get_currency().is_none());
    // The same market, another ticker: the currency does not fill, and
    // once the Valor number the row holds is derived, nothing else is
    // left to.
    let mut other_ticker = order(20, &[(IdType::Isin, HOLCIM)]);
    other_ticker.set_miccode(mic("XSWX"), true);
    other_ticker.set_ticker(Some(SmolStr::new("HOLNX")), true);
    assert!(registry.fill(&mut other_ticker));
    assert_eq!(
        other_ticker.get_securityids().get(&IdType::Valor),
        Some("1221405")
    );
    assert!(other_ticker.get_currency().is_none());
    assert!(!registry.fill(&mut other_ticker));
    // The same market and ticker: it fills; a stated one stands.
    let mut same = order(20, &[(IdType::Isin, HOLCIM)]);
    same.set_miccode(mic("XSWX"), true);
    assert!(registry.fill(&mut same));
    assert_eq!(same.get_currency().as_str(), "CHF");
    let mut priced = order(20, &[(IdType::Isin, HOLCIM)]);
    priced.set_miccode(mic("XSWX"), true);
    priced.set_currency(Ccy::new("USD").unwrap(), true);
    assert!(registry.fill(&mut priced));
    assert_eq!(priced.get_currency().as_str(), "USD");
    // A currency stated on the listing replaces whatever the time.
    let mut repriced = order(5, &[(IdType::Isin, HOLCIM)]);
    repriced.set_miccode(mic("XSWX"), true);
    repriced.set_currency(Ccy::new("USD").unwrap(), true);
    assert!(registry.learn(&repriced));
    assert_eq!(
        registry
            .get(HOLCIM)
            .unwrap()
            .currency()
            .map(|code| code.as_str()),
        Some("USD")
    );
    assert_eq!(registry.get(HOLCIM).unwrap().ticker(), Some("HOLN"));
}

#[test]
fn the_bound_refuses_a_new_isin_and_learning_passes_one_over() {
    crate::install::installed();
    let mut registry = IsinRegistry::new().with_max_instruments(1);
    assert_eq!(registry.max_instruments(), 1);
    registry
        .merge(entry(HOLCIM, Some(1), &[(IdType::Common, "A")]))
        .unwrap();
    let refused = registry
        .merge(entry(APPLE, Some(1), &[(IdType::Common, "B")]))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at most 1 instruments"), "{refused}");
    assert!(!registry.learn(&order(2, &[(IdType::Isin, APPLE), (IdType::Common, "B")])));
    assert!(
        registry.learn(&order(2, &[(IdType::Isin, HOLCIM), (IdType::Common, "C")])),
        "a known ISIN keeps learning"
    );
    assert_eq!(registry.len(), 1);
    // A row holds at most twelve equivalents; the ISIN is the key, never a
    // column; a `ZZ` ISIN names no instrument.
    let mut full = IsinEntry::new(isin(HOLCIM));
    for kind in [
        IdType::Quik,
        IdType::Dutch,
        IdType::Sicovam,
        IdType::Belgian,
        IdType::Common,
        IdType::ClearingHouse,
        IdType::FpmlSpec,
        IdType::Opra,
        IdType::FpmlUrl,
        IdType::Loc,
        IdType::MktAssigned,
        IdType::RedEntity,
    ] {
        full.set_code(kind, "X-1").unwrap();
    }
    assert_eq!(full.iter().count(), IsinRegistry::MAX_EQUIVALENTS);
    assert!(full.set_code(IdType::RedPair, "X-1").is_err());
    assert!(full.set_code(IdType::Isin, APPLE).is_err());
    assert!(full.set_code(IdType::ClOrdId, "X-1").is_err());
    assert!(
        full.set_code(IdType::Forex, "EUR/USD").is_err(),
        "a column of its own"
    );
    // Folded over a full row, a new type is passed over and moves nothing:
    // the registry stays clean and answers so; a held type restated moves.
    let mut seeded = IsinRegistry::new();
    seeded.merge(full.clone().with_updunix(Some(1))).unwrap();
    let mut holding = IsinRegistry::from_arrow_reader(seeded.into_arrow_reader().unwrap()).unwrap();
    assert!(!holding.is_dirty(), "loaded");
    assert!(
        !holding
            .merge(entry(HOLCIM, Some(99), &[(IdType::RedPair, "X-1")]))
            .unwrap()
    );
    assert!(!holding.is_dirty(), "nothing moved");
    assert_eq!(holding.get(HOLCIM).unwrap().updunix(), Some(1));
    // Learned, it moves the instant the instrument was met alone.
    assert!(holding.learn(&order(
        99,
        &[(IdType::Isin, HOLCIM), (IdType::RedPair, "X-1")]
    )));
    assert!(holding.is_dirty());
    let held = holding.get(HOLCIM).unwrap();
    assert_eq!((held.updunix(), held.lastunix()), (Some(1), Some(99)));
    assert_eq!(held.get(&IdType::RedPair), None);
    assert!(
        holding
            .merge(entry(HOLCIM, Some(99), &[(IdType::Quik, "X-2")]))
            .unwrap()
    );
    assert_eq!(holding.get(HOLCIM).unwrap().get(&IdType::Quik), Some("X-2"));
    assert_eq!(holding.get(HOLCIM).unwrap().updunix(), Some(99));
    let unknown = isin(&numbered("ZZ"));
    assert!(IsinRegistry::new().merge(IsinEntry::new(unknown)).is_err());
}

#[test]
fn an_entry_reads_back_from_its_scalar() {
    crate::install::installed();
    let entry = entry(
        HOLCIM,
        Some(7),
        &[(IdType::Ric, "HOLN.S"), (IdType::Valor, "1221405")],
    )
    .with_lastunix(Some(9))
    .with_cficode(cfi("ESVUFR"))
    .with_countrycode(Some(Country::new("CH").unwrap()))
    .with_forexcode(Some(Forex::new("EUR/CHF").unwrap()))
    .with_underlyingisin(Some(isin(APPLE)))
    .with_miccode(mic("XSWX"))
    .with_ticker(Some(SmolStr::new(" HOLN ")))
    .with_fisn(fisn("HOLCIM/REG SHS"))
    .with_currency(ccy("CHF"));
    assert_eq!(entry.ticker(), Some("HOLN"));
    assert_eq!(entry.lastunix(), Some(9));
    assert_eq!(entry.underlyingisin().map(Isin::as_str), Some(APPLE));
    assert_eq!(entry.fisn().map(Fisn::as_str), Some("HOLCIM/REG SHS"));
    assert_eq!(IsinEntry::from_scalar(&entry.into_scalar()).unwrap(), entry);
    assert_eq!(
        entry
            .into_scalar()
            .as_struct()
            .and_then(|cells| cells.get("fisn"))
            .and_then(|cell| cell.as_str()),
        Some("HOLCIM/REG SHS")
    );
    assert_eq!(
        entry
            .into_scalar()
            .as_struct()
            .and_then(|cells| cells.get("underlyingisin"))
            .and_then(|cell| cell.as_str()),
        Some(APPLE)
    );
    let columns: Vec<String> = IsinEntry::dtype()
        .as_fields()
        .unwrap()
        .iter()
        .map(|field| field.name().to_string())
        .collect();
    // The fourteen facts, the three stamps, the product category, the
    // short name and the origin currency among the instrument's, then the
    // thirty-two equivalents.
    assert_eq!(columns.len(), 46);
    assert_eq!(
        &columns[..15],
        [
            "isin",
            "updunix",
            "firstunix",
            "lastunix",
            "cficode",
            "countrycode",
            "forexcode",
            "underlyingisin",
            "eusipacode",
            "miccode",
            "ticker",
            "fisn",
            "currency",
            "origccy",
            "cusip"
        ]
    );
    assert!(!IsinEntry::field().is_nullable());
    let types: Vec<String> = IsinEntry::dtype()
        .as_fields()
        .unwrap()
        .iter()
        .take(14)
        .map(|field| field.dtype().to_string())
        .collect();
    assert_eq!(
        types,
        [
            "isin",
            "datetime64(ns,\"UTC\")",
            "datetime64(ns,\"UTC\")",
            "datetime64(ns,\"UTC\")",
            "cfi",
            "country",
            "forex",
            "isin",
            "int32",
            "mic",
            "utf8",
            "fisn",
            "ccy",
            "ccy"
        ]
    );
}

/// The underlying is an instrument fact: a real ISIN other than the row's
/// own fills and replaces on any market and a new listing holds it; the
/// row's own ISIN or a typo states nothing; `learn` never states one, since
/// what a FIX message names as its underlying is the lifecycle's reading.
#[test]
fn the_underlying_is_an_instrument_fact_merged_by_the_update_rule() {
    crate::install::installed();
    let holcim = || IsinEntry::new(isin(HOLCIM));
    assert_eq!(
        holcim()
            .with_underlyingisin(Some(isin(HOLCIM)))
            .underlyingisin(),
        None,
        "an instrument is not written on itself"
    );
    let mut registry = IsinRegistry::new();
    assert!(
        registry
            .merge(
                holcim()
                    .with_miccode(mic("XSWX"))
                    .with_ticker(Some(SmolStr::new("HOLN")))
            )
            .unwrap()
    );
    let underlying = |registry: &IsinRegistry| {
        registry
            .get(HOLCIM)
            .and_then(|row| row.underlyingisin().map(|code| code.as_str().to_owned()))
    };
    assert!(
        !registry
            .merge(holcim().with_underlyingisin(Some(isin(HOLCIM))))
            .unwrap(),
        "the row's own ISIN states nothing"
    );
    assert_eq!(underlying(&registry), None);
    assert!(
        registry
            .merge(holcim().with_underlyingisin(Some(isin(APPLE))))
            .unwrap(),
        "a real ISIN fills"
    );
    assert_eq!(underlying(&registry).as_deref(), Some(APPLE));
    assert!(
        !registry
            .merge(holcim().with_underlyingisin(Some(isin(APPLE))))
            .unwrap(),
        "the same value moves nothing"
    );
    assert!(
        !registry.merge(holcim()).unwrap(),
        "a statement of none moves nothing"
    );
    // A typo is dropped with a warning and moves nothing.
    assert!(
        !registry
            .merge(holcim().with_underlyingisin(Some(isin("US0378331006"))))
            .unwrap()
    );
    assert_eq!(underlying(&registry).as_deref(), Some(APPLE));
    // A different real ISIN replaces whatever the time.
    assert!(
        registry
            .merge(
                holcim()
                    .with_updunix(Some(1))
                    .with_underlyingisin(Some(isin(NOVARTIS)))
            )
            .unwrap()
    );
    assert_eq!(underlying(&registry).as_deref(), Some(NOVARTIS));
    // A listing on another market holds it too: an instrument fact.
    assert!(
        registry
            .merge(
                holcim()
                    .with_miccode(mic("XLON"))
                    .with_ticker(Some(SmolStr::new("HOLNL")))
            )
            .unwrap()
    );
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.miccode().map(Mic::as_str), Some("XLON"));
    assert_eq!(row.ticker(), Some("HOLNL"));
    for row in registry.listings(HOLCIM) {
        assert_eq!(row.underlyingisin().map(Isin::as_str), Some(NOVARTIS));
    }
    // `learn` never states one.
    let mut fresh = IsinRegistry::new();
    assert!(fresh.learn(&order(
        5,
        &[(IdType::Isin, APPLE), (IdType::Cusip, "037833100")]
    )));
    assert_eq!(fresh.get(APPLE).unwrap().underlyingisin(), None);
    // The named struct reads it back, and the ordered row carries it.
    let entry = holcim().with_underlyingisin(Some(isin(APPLE)));
    assert_eq!(IsinEntry::from_scalar(&entry.into_scalar()).unwrap(), entry);
    let named = yggdryl::Scalar::from_struct([
        (SmolStr::new_static("isin"), yggdryl::Scalar::from(HOLCIM)),
        (
            SmolStr::new_static("underlyingisin"),
            yggdryl::Scalar::from(APPLE),
        ),
    ])
    .unwrap();
    assert_eq!(IsinEntry::from_scalar(&named).unwrap(), entry);
}

/// The EUSIPA product category is an instrument fact: a category of its
/// shape fills and replaces on any market whatever either map lists, a new
/// listing holds it, `learn` never states one, and it crosses the
/// row's scalar as its number - a number of no category's shape read as
/// none.
#[test]
fn the_product_category_is_an_instrument_fact_merged_by_the_update_rule() {
    crate::install::installed();
    let holcim = || IsinEntry::new(isin(HOLCIM));
    let category = |code: u16| Some(Eusipa::new(code).unwrap());
    let mut registry = IsinRegistry::new();
    assert!(
        registry
            .merge(
                holcim()
                    .with_miccode(mic("XSWX"))
                    .with_ticker(Some(SmolStr::new("HOLN")))
            )
            .unwrap()
    );
    let held = |registry: &IsinRegistry| registry.get(HOLCIM).and_then(IsinEntry::eusipacode);
    assert_eq!(held(&registry), None);
    assert!(
        registry
            .merge(holcim().with_eusipacode(category(2300)))
            .unwrap(),
        "a category fills"
    );
    assert_eq!(held(&registry), category(2300));
    assert!(
        !registry
            .merge(holcim().with_eusipacode(category(2300)))
            .unwrap(),
        "the same category moves nothing"
    );
    assert!(
        !registry.merge(holcim()).unwrap(),
        "a statement of none moves nothing"
    );
    // A category neither map lists is a category: the maps move on.
    assert!(
        registry
            .merge(holcim().with_eusipacode(category(2301)))
            .unwrap()
    );
    assert_eq!(held(&registry), category(2301));
    assert!(
        registry
            .merge(
                holcim()
                    .with_updunix(Some(1))
                    .with_eusipacode(category(1260))
            )
            .unwrap(),
        "a different category replaces whatever the time"
    );
    // A listing on another market holds it too.
    assert!(
        registry
            .merge(
                holcim()
                    .with_miccode(mic("XLON"))
                    .with_ticker(Some(SmolStr::new("HOLNL")))
            )
            .unwrap()
    );
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.miccode().map(Mic::as_str), Some("XLON"));
    assert_eq!(row.eusipacode(), category(1260));
    assert_eq!(
        registry
            .get_listing(HOLCIM, &mic("XSWX").unwrap())
            .and_then(IsinEntry::eusipacode),
        category(1260)
    );
    // `learn` never states one.
    let mut fresh = IsinRegistry::new();
    assert!(fresh.learn(&order(
        5,
        &[(IdType::Isin, APPLE), (IdType::Cusip, "037833100")]
    )));
    assert_eq!(fresh.get(APPLE).unwrap().eusipacode(), None);
    // The scalar carries the number, the named struct and the ordered row
    // read it back, and a number of no category's shape reads as none.
    let entry = holcim().with_eusipacode(category(2300));
    let scalar = entry.into_scalar();
    assert_eq!(
        scalar.as_struct().and_then(|cells| cells.get("eusipacode")),
        Some(&yggdryl::Scalar::from(2300_i32))
    );
    assert_eq!(IsinEntry::from_scalar(&scalar).unwrap(), entry);
    let named = |code: yggdryl::Scalar| {
        yggdryl::Scalar::from_struct([
            (SmolStr::new_static("isin"), yggdryl::Scalar::from(HOLCIM)),
            (SmolStr::new_static("eusipacode"), code),
        ])
        .unwrap()
    };
    assert_eq!(
        IsinEntry::from_scalar(&named(yggdryl::Scalar::from(2300_i64))).unwrap(),
        entry
    );
    for unshaped in [3100_i64, -2300, 70_000] {
        assert_eq!(
            IsinEntry::from_scalar(&named(yggdryl::Scalar::from(unshaped)))
                .unwrap()
                .eusipacode(),
            None,
            "{unshaped}"
        );
    }
    assert!(
        IsinEntry::from_scalar(&named(yggdryl::Scalar::from(i64::MAX))).is_err(),
        "a number no int32 holds is the column's refusal"
    );
}

/// A golden file states the product category under EUSIPA's or the SSPA's
/// name - Euronext's `EUSIPA_Code`, a bare `SSPA` - as a number or as text
/// of one; a cell of no category's shape lands as none, and a column naming
/// the category's name is passed over.
#[test]
fn a_golden_file_states_the_product_category_by_either_maps_name() {
    crate::install::installed();
    let load = |name: &str, cells: Arc<dyn arrow_array::Array>| {
        let schema = Arc::new(Schema::new(vec![
            ArrowField::new("ISIN", ArrowType::Utf8, false),
            ArrowField::new(name, cells.data_type().clone(), true),
            ArrowField::new("EUSIPA_Name", ArrowType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(vec![HOLCIM, APPLE, NOVARTIS])),
                cells,
                Arc::new(StringArray::from(vec![
                    Some("Constant Leverage Certificate"),
                    None,
                    None,
                ])),
            ],
        )
        .unwrap();
        IsinRegistry::from_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch])).unwrap()
    };
    let categories = |registry: &IsinRegistry| {
        [HOLCIM, APPLE, NOVARTIS].map(|key| {
            registry
                .get(key)
                .and_then(IsinEntry::eusipacode)
                .map(|code| code.code())
        })
    };
    for name in [
        "EUSIPA_Code",
        "eusipa",
        "EUSIPACategory",
        "SSPA",
        "sspa_code",
        "SSPACategory",
    ] {
        let numbers = load(
            name,
            Arc::new(Int32Array::from(vec![Some(2300), Some(3100), None])),
        );
        assert_eq!(categories(&numbers), [Some(2300), None, None], "{name}");
        let text = load(
            name,
            Arc::new(StringArray::from(vec![
                Some("2300"),
                Some("1260"),
                Some(""),
            ])),
        );
        assert_eq!(categories(&text), [Some(2300), Some(1260), None], "{name}");
    }
    let unsigned = load(
        "eusipacode",
        Arc::new(UInt16Array::from(vec![Some(2205), None, Some(999)])),
    );
    assert_eq!(categories(&unsigned), [Some(2205), None, None]);
    let wide = load(
        "EUSIPA",
        Arc::new(Int64Array::from(vec![Some(2300), Some(-1), Some(i64::MAX)])),
    );
    assert_eq!(categories(&wide), [Some(2300), None, None]);
}

#[test]
fn a_registry_round_trips_an_ipc_file_through_the_record_surface() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    registry
        .merge(
            entry(
                HOLCIM,
                Some(7),
                &[(IdType::Ric, "HOLN.S"), (IdType::Valor, "1221405")],
            )
            .with_cficode(cfi("ESVUFR"))
            .with_countrycode(Some(Country::new("CH").unwrap()))
            .with_miccode(mic("XSWX"))
            .with_currency(ccy("CHF")),
        )
        .unwrap();
    registry
        .merge(
            entry(APPLE, None, &[(IdType::Cusip, "037833100")])
                .with_forexcode(Some(Forex::new("USD/CHF").unwrap()))
                .with_origccy(ccy("USD")),
        )
        .unwrap();
    let mut handle = Buffer::new().with_media_type(yggdryl::MimeType::ARROW_STREAM.into());
    let options = handle
        .record_options()
        .unwrap()
        .with_field(IsinEntry::field());
    handle
        .write_arrow_reader(
            registry.into_arrow_reader().unwrap(),
            IOMode::Overwrite,
            &options,
        )
        .unwrap();
    let mut back = IsinRegistry::new();
    assert_eq!(back.extend_from_handle(&handle).unwrap(), 2);
    assert!(back.iter().eq(registry.iter()));
    assert!(back.is_dirty(), "a fold that moved marks the registry");
    // Folding the same rows again moves nothing.
    let mut again = IsinRegistry::new();
    again.extend_from_handle(&handle).unwrap();
    assert_eq!(again.extend_from_handle(&handle).unwrap(), 2);
    assert!(again.iter().eq(back.iter()));
}

/// Holcim at `unix` with a RIC and a valor on SIX, and Apple undated with
/// a CUSIP.
fn snapshot(unix: i64) -> IsinRegistry {
    let mut registry = IsinRegistry::new();
    registry
        .merge(
            entry(
                HOLCIM,
                Some(unix),
                &[(IdType::Ric, "HOLN.S"), (IdType::Valor, "1221405")],
            )
            .with_cficode(cfi("ESVUFR"))
            .with_miccode(mic("XSWX")),
        )
        .unwrap();
    registry
        .merge(entry(APPLE, None, &[(IdType::Cusip, "037833100")]))
        .unwrap();
    registry
}

/// `registry` written to `handle` by `mode` under the handle's own options.
fn save(handle: &mut impl IOMedia, registry: &IsinRegistry, mode: IOMode, merge_by: bool) {
    let mut options = handle
        .record_options()
        .unwrap()
        .with_field(IsinEntry::field());
    if merge_by {
        options = options.with_merge_by(["isin"]).unwrap();
    }
    handle
        .write_arrow_reader(registry.into_arrow_reader().unwrap(), mode, &options)
        .unwrap();
}

#[cfg(feature = "parquet")]
#[test]
fn a_registry_round_trips_parquet_through_the_record_surface() {
    crate::install::installed();
    let registry = snapshot(7);
    let mut handle = Buffer::new().with_media_type(yggdryl::MimeType::PARQUET.into());
    save(&mut handle, &registry, IOMode::Overwrite, false);
    let mut back = IsinRegistry::new();
    assert_eq!(back.extend_from_handle(&handle).unwrap(), 2);
    assert!(back.iter().eq(registry.iter()));
    assert_eq!(back.get(HOLCIM).unwrap().get(&IdType::Ric), Some("HOLN.S"));
}

/// A merge by `isin` upserts: a row of a stored ISIN is replaced by the
/// incoming one, a new ISIN appended, every other row kept.
#[test]
fn a_merge_by_isin_upserts_the_stored_rows() {
    crate::install::installed();
    let mut handle = Buffer::new().with_media_type(yggdryl::MimeType::ARROW_STREAM.into());
    save(&mut handle, &snapshot(7), IOMode::Overwrite, false);
    let mut incoming = IsinRegistry::new();
    incoming
        .merge(entry(HOLCIM, Some(20), &[(IdType::Common, "C-2")]))
        .unwrap();
    incoming
        .merge(entry(NOVARTIS, Some(20), &[(IdType::Valor, "1200526")]))
        .unwrap();
    save(&mut handle, &incoming, IOMode::Merge, true);
    let mut back = IsinRegistry::new();
    back.extend_from_handle(&handle).unwrap();
    assert_eq!(back.len(), 3);
    let holcim = back.get(HOLCIM).unwrap();
    assert_eq!(holcim.get(&IdType::Common), Some("C-2"));
    assert_eq!(
        holcim.get(&IdType::Ric),
        None,
        "the stored row was replaced"
    );
    assert_eq!(holcim.updunix(), Some(20));
    assert_eq!(
        back.get(APPLE).unwrap().get(&IdType::Cusip),
        Some("037833100")
    );
    assert_eq!(
        back.get(NOVARTIS).unwrap().get(&IdType::Valor),
        Some("1200526")
    );
}

/// A folder of parts reads as one stream, its rows of one ISIN folded in
/// part order whichever part holds them: a part written by an overwrite
/// and grown by an append of another market's listing, then a second part
/// of older rows of no market, whose instrument facts still fill and
/// replace on every listing and whose listing facts land on none.
#[test]
fn a_folder_of_parts_loads_in_part_order() {
    crate::install::installed();
    let mut root = yggdryl::local::LocalFolder::temporary()
        .unwrap()
        .path()
        .unwrap();
    root.push(format!("yggdryl-isin-registry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let options = || {
        RecordOptions::for_mime_type(&yggdryl::MimeType::ARROW_STREAM)
            .unwrap()
            .with_field(IsinEntry::field())
    };
    let part = |name: &str| Holder::folder(&root).unwrap().child_by_path(name).unwrap();

    let mut first = part("part-0.arrows");
    first
        .write_arrow_reader(
            snapshot(10).into_arrow_reader().unwrap(),
            IOMode::Overwrite,
            &options(),
        )
        .unwrap();
    let mut later = IsinRegistry::new();
    later
        .merge(entry(HOLCIM, Some(30), &[(IdType::Ric, "HOLN.L")]).with_miccode(mic("XLON")))
        .unwrap();
    first
        .write_arrow_reader(
            later.into_arrow_reader().unwrap(),
            IOMode::Append,
            &options(),
        )
        .unwrap();
    first.flush().unwrap();
    let mut older = IsinRegistry::new();
    older
        .merge(entry(
            HOLCIM,
            Some(5),
            &[(IdType::Ric, "HOLN.Z"), (IdType::Common, "C-5")],
        ))
        .unwrap();
    let mut second = part("part-1.arrows");
    second
        .write_arrow_reader(
            older.into_arrow_reader().unwrap(),
            IOMode::Overwrite,
            &options(),
        )
        .unwrap();
    second.flush().unwrap();

    let folder = Holder::folder(&root).unwrap();
    let mut loaded = IsinRegistry::new();
    assert_eq!(
        loaded
            .extend_from_arrow_reader(folder.read_arrow_reader(&options()).unwrap())
            .unwrap(),
        4
    );
    assert_eq!((loaded.len(), loaded.rows()), (2, 3));
    let listings = loaded.listings(HOLCIM);
    let markets: Vec<_> = listings
        .iter()
        .map(|row| row.miccode().map(Mic::as_str))
        .collect();
    assert_eq!(
        markets,
        [Some("XLON"), Some("XSWX")],
        "the first part's appended listing is a second one"
    );
    for row in listings {
        assert_eq!(row.updunix(), Some(30), "the stamp is the latest");
        assert_eq!(
            row.get(&IdType::Common),
            Some("C-5"),
            "filled on every listing"
        );
        assert_eq!(row.get(&IdType::Valor), Some("1221405"));
        assert_eq!(row.cficode().map(|code| code.as_str()), Some("ESVUFR"));
    }
    let [london, swiss] = listings else {
        panic!("two listings")
    };
    assert_eq!(
        (london.get(&IdType::Ric), swiss.get(&IdType::Ric)),
        (Some("HOLN.L"), Some("HOLN.S")),
        "the second part's RIC names no market of two, and lands on neither"
    );
    assert_eq!(london.currency().map(|code| code.as_str()), Some("GBP"));
    assert_eq!(swiss.currency().map(|code| code.as_str()), Some("CHF"));
    assert!(loaded.get(APPLE).is_some());
    // The folder's own record stream is the same door.
    let mut from_folder = IsinRegistry::new();
    from_folder.extend_from_handle(&folder).unwrap();
    assert!(from_folder.iter().eq(loaded.iter()));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_missing_store_reads_as_the_empty_registry() {
    crate::install::installed();
    let missing = Buffer::new().with_media_type(yggdryl::MimeType::ARROW_STREAM.into());
    let mut registry = IsinRegistry::new();
    assert_eq!(registry.extend_from_handle(&missing).unwrap(), 0);
    assert!(registry.is_empty());
    assert!(!registry.is_dirty());
}

#[test]
fn a_flat_golden_file_loads_by_the_columns_its_names_spell() {
    crate::install::installed();
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("ISIN", ArrowType::Utf8, false),
        ArrowField::new("RIC", ArrowType::Utf8, true),
        ArrowField::new("CFI", ArrowType::Utf8, true),
        ArrowField::new("BloombergSymbol", ArrowType::Utf8, true),
        ArrowField::new("MIC", ArrowType::Utf8, true),
        ArrowField::new("Country", ArrowType::Utf8, true),
        ArrowField::new("Currency", ArrowType::Utf8, true),
        ArrowField::new("CcyPair", ArrowType::Utf8, true),
        ArrowField::new("rank", ArrowType::Int64, true),
        ArrowField::new("UnderlyingISIN", ArrowType::Utf8, true),
        ArrowField::new("ValorSymbol", ArrowType::Utf8, true),
        ArrowField::new("X-SWX-VALOR", ArrowType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from(vec![HOLCIM, APPLE])),
            Arc::new(StringArray::from(vec![Some("HOLN.S"), None])),
            Arc::new(StringArray::from(vec![Some("ESVUFR"), None])),
            Arc::new(StringArray::from(vec![Some("HOLN SW Equity"), None])),
            Arc::new(StringArray::from(vec![Some("XSWX"), None])),
            Arc::new(StringArray::from(vec![Some("LI"), None])),
            Arc::new(StringArray::from(vec![Some("CHF"), None])),
            Arc::new(StringArray::from(vec![None, Some("USD/CHF")])),
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some(APPLE), None])),
            Arc::new(StringArray::from(vec![Some("HOLN"), None])),
            Arc::new(StringArray::from(vec![Some("1221405"), None])),
        ],
    )
    .unwrap();
    let registry =
        IsinRegistry::from_arrow_reader(yggdryl::arrow::batch_reader(Arc::clone(&schema), [batch]))
            .unwrap();
    assert_eq!(registry.len(), 2);
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(row.get(&IdType::Bloomberg), Some("HOLN SW Equity"));
    assert_eq!(row.cficode().map(|code| code.as_str()), Some("ESVUFR"));
    assert_eq!(row.miccode().map(|code| code.as_str()), Some("XSWX"));
    assert_eq!(row.countrycode().map(|code| code.as_str()), Some("LI"));
    assert_eq!(row.currency().map(|code| code.as_str()), Some("CHF"));
    // A key naming an underlying's ISIN lands in `underlyingisin`; SIX's
    // symbol is the exchange symbol of the row's listing, and the vendor's
    // source spelling its Valor number.
    assert_eq!(row.underlyingisin().map(Isin::as_str), Some(APPLE));
    assert_eq!(row.get(&IdType::ExchSymb), Some("HOLN"));
    assert_eq!(row.get(&IdType::Valor), Some("1221405"));
    let apple = registry.get(APPLE).unwrap();
    assert_eq!(
        apple.iter().collect::<Vec<_>>(),
        [(&IdType::Cusip, "037833100")],
        "the CUSIP its ISIN embeds, derived; nothing else stated"
    );
    assert_eq!(apple.forexcode().map(|pair| pair.as_str()), Some("USD/CHF"));

    // Two columns naming one fact, and no ISIN at all, are refused.
    let two = Arc::new(Schema::new(vec![
        ArrowField::new("ISIN", ArrowType::Utf8, false),
        ArrowField::new("isin_number", ArrowType::Utf8, false),
    ]));
    let refused = IsinRegistry::from_arrow_reader(yggdryl::arrow::batch_reader(two, []))
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("ISIN") && refused.contains("isin_number"),
        "{refused}"
    );
    let none = Arc::new(Schema::new(vec![ArrowField::new(
        "RIC",
        ArrowType::Utf8,
        true,
    )]));
    assert!(IsinRegistry::from_arrow_reader(yggdryl::arrow::batch_reader(none, [])).is_err());
}

/// One flat `utf8` batch of `columns`, each a name and its cells.
fn flat(columns: &[(&str, Vec<Option<&str>>)]) -> yggdryl::arrow::BatchReader {
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(name, _)| ArrowField::new(*name, ArrowType::Utf8, true))
            .collect::<Vec<_>>(),
    ));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        columns
            .iter()
            .map(|(_, cells)| Arc::new(StringArray::from(cells.clone())) as _)
            .collect(),
    )
    .unwrap();
    yggdryl::arrow::batch_reader(schema, [batch])
}

/// A `utf8` equivalent its type refuses is refused on its row and column;
/// a typed code column's unreadable cell lands null and a ticker outside
/// one to sixty-four bytes is stored as none, the rest of the row kept.
#[test]
fn a_load_refuses_a_utf8_code_its_type_refuses_on_its_row_and_column() {
    crate::install::installed();
    let refused = IsinRegistry::from_arrow_reader(flat(&[
        ("isin", vec![Some(HOLCIM), Some(APPLE)]),
        ("wkn", vec![Some("716460"), Some("TOOLONGWKN")]),
    ]))
    .unwrap_err()
    .to_string();
    assert!(refused.contains("$[1].wkn"), "{refused}");

    let long = "R".repeat(200);
    let ticker = "T".repeat(65);
    let registry = IsinRegistry::from_arrow_reader(flat(&[
        ("isin", vec![Some(HOLCIM)]),
        ("ric", vec![Some(long.as_str())]),
        ("ticker", vec![Some(ticker.as_str())]),
        ("valor", vec![Some("1221405")]),
        ("country", vec![Some("Switzerland")]),
        ("currency", vec![Some("XXX")]),
    ]))
    .unwrap();
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Ric), None);
    assert_eq!(row.ticker(), None);
    assert_eq!(row.countrycode(), None);
    assert_eq!(
        row.currency(),
        None,
        "XXX states none, and a row of no market takes no default"
    );
    assert_eq!(row.get(&IdType::Valor), Some("1221405"));
}

/// An LEI and a DTI equivalent are typed by their own codes, so a store
/// declares them and a round trip keeps them; a store whose column is
/// `utf8` still loads, its cells cast into the code, a cell that is not the
/// code's canonical spelling landing null.
#[test]
fn an_lei_and_a_dti_column_is_its_own_code_and_a_utf8_one_still_loads() {
    crate::install::installed();
    let declared = IsinEntry::dtype();
    for (name, dtype) in [
        ("lei", yggdryl::DataType::lei()),
        ("dti", yggdryl::DataType::dti()),
    ] {
        let column = declared
            .as_fields()
            .unwrap()
            .iter()
            .find(|field| field.name() == name)
            .unwrap_or_else(|| panic!("the {name} column"));
        assert_eq!(column.dtype(), &dtype, "{name}");
    }
    let mut registry = IsinRegistry::new();
    registry
        .merge(
            IsinEntry::new(isin(APPLE))
                .try_with_code(IdType::Lei, "hwupkr0mpou8fgxbt394")
                .unwrap()
                .try_with_code(IdType::Dti, "x9j9k872s")
                .unwrap(),
        )
        .unwrap();
    let read = IsinRegistry::from_arrow_reader(registry.into_arrow_reader().unwrap()).unwrap();
    let row = read.get(APPLE).unwrap();
    assert_eq!(row.get(&IdType::Lei), Some("HWUPKR0MPOU8FGXBT394"));
    assert_eq!(row.get(&IdType::Dti), Some("X9J9K872S"));

    let stored = IsinRegistry::from_arrow_reader(flat(&[
        ("isin", vec![Some(APPLE), Some(HOLCIM)]),
        (
            "lei",
            vec![Some("HWUPKR0MPOU8FGXBT394"), Some("hwupkr0mpou8fgxbt394")],
        ),
        ("dti", vec![Some("NOT-A-DTI"), None]),
    ]))
    .unwrap();
    let apple = stored.get(APPLE).unwrap();
    assert_eq!(apple.get(&IdType::Lei), Some("HWUPKR0MPOU8FGXBT394"));
    // The column lands as the code through the safe cast: a cell that is
    // not its canonical spelling lands null, the rest of the row kept.
    assert_eq!(apple.get(&IdType::Dti), None);
    assert_eq!(stored.get(HOLCIM).unwrap().get(&IdType::Lei), None);
}

/// A row stating more than twelve equivalents is refused, naming the row.
#[test]
fn a_load_refuses_a_row_past_twelve_equivalents() {
    crate::install::installed();
    let kinds = [
        IdType::Quik,
        IdType::Dutch,
        IdType::Sicovam,
        IdType::Belgian,
        IdType::Common,
        IdType::ClearingHouse,
        IdType::FpmlSpec,
        IdType::Opra,
        IdType::FpmlUrl,
        IdType::Loc,
        IdType::MktAssigned,
        IdType::RedEntity,
        IdType::RedPair,
    ];
    let mut columns = vec![("isin", vec![Some(HOLCIM)])];
    columns.extend(kinds.iter().map(|kind| (kind.as_str(), vec![Some("X-1")])));
    let refused = IsinRegistry::from_arrow_reader(flat(&columns))
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("$[0].") && refused.contains("at most 12 equivalents"),
        "{refused}"
    );
    // Twelve load.
    columns.pop();
    assert_eq!(
        IsinRegistry::from_arrow_reader(flat(&columns))
            .unwrap()
            .get(HOLCIM)
            .unwrap()
            .iter()
            .count(),
        IsinRegistry::MAX_EQUIVALENTS
    );
}

/// The short name is an instrument fact: learned where an element states
/// it as a `fisn` security identifier - never where it was only derived -
/// filled into one naming the ISIN on any market as a derived identifier,
/// replaced by another whatever the time, held by a new listing, and
/// carried by the scalar, the snapshot stream and a golden file's column.
#[test]
fn the_short_name_is_an_instrument_fact_learned_filled_and_read_back() {
    crate::install::installed();
    const NAME: &str = "APPLE INC/SH SH";
    let mut registry = IsinRegistry::new();
    let mut stated = order(10, &[(IdType::Isin, APPLE), (IdType::Fisn, NAME)]);
    stated.set_miccode(mic("XNAS"), true);
    stated.set_ticker(Some(SmolStr::new("AAPL")), true);
    assert!(registry.learn(&stated));
    assert_eq!(
        registry.get(APPLE).unwrap().fisn().map(Fisn::as_str),
        Some(NAME)
    );
    assert!(!registry.learn(&stated), "nothing new");

    // Filled into an element naming the ISIN, derived, on any market.
    let mut named = order(20, &[(IdType::Isin, APPLE)]);
    assert!(registry.fill(&mut named));
    assert_eq!(named.get_securityids().get(&IdType::Fisn), Some(NAME));
    assert!(named.get_securityids().is_derived(&IdType::Fisn));
    let mut elsewhere = order(20, &[(IdType::Isin, APPLE)]);
    elsewhere.set_miccode(mic("XLON"), true);
    assert!(registry.fill(&mut elsewhere));
    assert_eq!(elsewhere.get_securityids().get(&IdType::Fisn), Some(NAME));
    // What a fill derived is never learned.
    let mut fresh = IsinRegistry::new();
    fresh.learn(&named);
    assert_eq!(fresh.get(APPLE).and_then(IsinEntry::fisn), None);
    // A stated one stands over the row's.
    let mut own = order(
        20,
        &[(IdType::Isin, APPLE), (IdType::Fisn, "APPLE INC/COM")],
    );
    registry.fill(&mut own);
    assert_eq!(
        own.get_securityids().get(&IdType::Fisn),
        Some("APPLE INC/COM")
    );

    // Another replaces whatever the time; a new listing holds it.
    assert!(
        registry
            .merge(
                IsinEntry::new(isin(APPLE))
                    .with_updunix(Some(1))
                    .with_fisn(fisn("APPLE INC/COM"))
            )
            .unwrap()
    );
    assert!(
        registry
            .merge(
                IsinEntry::new(isin(APPLE))
                    .with_miccode(mic("XLON"))
                    .with_ticker(Some(SmolStr::new("0R2V")))
            )
            .unwrap()
    );
    let row = registry.get(APPLE).unwrap();
    assert_eq!(row.miccode().map(Mic::as_str), Some("XLON"));
    assert_eq!(row.fisn().map(Fisn::as_str), Some("APPLE INC/COM"));
    assert!(
        !registry
            .merge(IsinEntry::new(isin(APPLE)).with_fisn(fisn("apple inc/com")))
            .unwrap(),
        "the same name, folded upper case, moves nothing"
    );

    // The scalar and the snapshot stream carry it.
    let row = registry.get(APPLE).unwrap();
    assert_eq!(IsinEntry::from_scalar(&row.into_scalar()).unwrap(), *row);
    let back = IsinRegistry::from_arrow_reader(registry.into_arrow_reader().unwrap()).unwrap();
    assert!(back.iter().eq(registry.iter()));
    assert_eq!(
        back.get(APPLE).unwrap().fisn().map(Fisn::as_str),
        Some("APPLE INC/COM")
    );
    // A golden file names it by any of its spellings.
    for name in [
        "FISN",
        "fisn_code",
        "ShortName",
        "FinancialInstrumentShortName",
    ] {
        let loaded = IsinRegistry::from_arrow_reader(flat(&[
            ("ISIN", vec![Some(APPLE)]),
            (name, vec![Some(NAME)]),
        ]))
        .unwrap();
        assert_eq!(
            loaded.get(APPLE).unwrap().fisn().map(Fisn::as_str),
            Some(NAME),
            "{name}"
        );
    }
}

/// A row the registry folds holds the national number its ISIN embeds in
/// its equivalent column where it states none - a CUSIP for `US`, a SEDOL
/// for `GB` behind `00`, a Valor number for `CH`, a WKN for `DE` behind
/// `000` - and a stated code is never replaced by the derivation; a row
/// built by hand and never folded holds only what it was given.
#[test]
fn a_folded_row_holds_the_national_number_its_isin_embeds() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    for (key, kind, code) in [
        (APPLE, IdType::Cusip, "037833100"),
        (DIAGEO, IdType::Sedol, "0237400"),
        (HOLCIM, IdType::Valor, "1221405"),
        (SAP, IdType::Wkn, "716460"),
    ] {
        assert!(registry.merge(IsinEntry::new(isin(key))).unwrap(), "{key}");
        let row = registry.get(key).unwrap();
        assert_eq!(row.get(&kind), Some(code), "{key}");
        assert_eq!(row.iter().count(), 1, "{key}: that one code");
    }
    let french = numbered("FR");
    registry.merge(IsinEntry::new(isin(&french))).unwrap();
    assert!(
        registry.get(&french).unwrap().iter().next().is_none(),
        "no scheme this crate checks"
    );
    assert!(
        IsinEntry::new(isin(APPLE)).get(&IdType::Cusip).is_none(),
        "never folded"
    );

    // A stated code stands, created or moved.
    const MICROSOFT: &str = "594918104";
    let mut stated = IsinRegistry::new();
    stated
        .merge(entry(APPLE, Some(1), &[(IdType::Cusip, MICROSOFT)]))
        .unwrap();
    assert_eq!(
        stated.get(APPLE).unwrap().get(&IdType::Cusip),
        Some(MICROSOFT)
    );
    assert!(
        stated
            .merge(entry(APPLE, Some(2), &[(IdType::Common, "C-1")]))
            .unwrap()
    );
    assert_eq!(
        stated.get(APPLE).unwrap().get(&IdType::Cusip),
        Some(MICROSOFT)
    );
    // A row with no room left passes the derivation over.
    let full = [
        IdType::Quik,
        IdType::Dutch,
        IdType::Sicovam,
        IdType::Belgian,
        IdType::Common,
        IdType::ClearingHouse,
        IdType::FpmlSpec,
        IdType::Opra,
        IdType::FpmlUrl,
        IdType::Loc,
        IdType::MktAssigned,
        IdType::RedEntity,
    ]
    .into_iter()
    .fold(IsinEntry::new(isin(APPLE)), |entry, kind| {
        entry.try_with_code(kind, "X-1").unwrap()
    });
    let mut bounded = IsinRegistry::new();
    bounded.merge(full).unwrap();
    assert_eq!(bounded.get(APPLE).unwrap().get(&IdType::Cusip), None);

    // The derived SEDOL is a listing code: filled on the row's market
    // alone.
    let mut listed = IsinRegistry::new();
    listed
        .merge(IsinEntry::new(isin(DIAGEO)).with_miccode(mic("XLON")))
        .unwrap();
    let mut paris = order(10, &[(IdType::Isin, DIAGEO)]);
    paris.set_miccode(mic("XPAR"), true);
    assert!(!listed.fill(&mut paris), "another market's listing code");
    assert_eq!(paris.get_securityids().get(&IdType::Sedol), None);
    let mut london = order(10, &[(IdType::Isin, DIAGEO)]);
    london.set_miccode(mic("XLON"), true);
    assert!(listed.fill(&mut london));
    assert_eq!(
        london.get_securityids().get(&IdType::Sedol),
        Some("0237400")
    );
    assert!(london.get_securityids().is_derived(&IdType::Sedol));
}

/// A row the registry folds with no currency takes its listing's default,
/// the legal tender of the country its market is in - its market alone,
/// never its ISIN's country - and a row of no market, or of a market of no
/// single country, takes none; a stated currency stands, a later statement
/// replaces a default as it replaces any value, a market arriving later
/// sets it, and a listing on another market takes that market's. A default
/// never travels as a statement: a stored row read back over two listings
/// moves no currency it did not state. A ticker-keyed element on the
/// listing is filled with it.
#[test]
fn a_folded_row_defaults_its_currency_to_its_markets_country() {
    crate::install::installed();
    let currency = |registry: &IsinRegistry, key: &str| {
        registry
            .get(key)
            .and_then(IsinEntry::currency)
            .map(|code| code.as_str().to_owned())
    };
    let mut registry = IsinRegistry::new();
    registry
        .merge(
            IsinEntry::new(isin(DIAGEO))
                .with_miccode(mic("XLON"))
                .with_ticker(Some(SmolStr::new("DGE"))),
        )
        .unwrap();
    assert_eq!(currency(&registry, DIAGEO).as_deref(), Some("GBP"));
    let french = numbered("FR");
    registry.merge(IsinEntry::new(isin(&french))).unwrap();
    assert_eq!(
        currency(&registry, &french),
        None,
        "no market: a currency is a listing's"
    );
    // A market arriving later sets it.
    assert!(
        registry
            .merge(IsinEntry::new(isin(&french)).with_miccode(mic("XPAR")))
            .unwrap()
    );
    assert_eq!(currency(&registry, &french).as_deref(), Some("EUR"));
    registry
        .merge(IsinEntry::new(isin(APPLE)).with_miccode(mic("XETR")))
        .unwrap();
    assert_eq!(
        currency(&registry, APPLE).as_deref(),
        Some("EUR"),
        "the market's country only, never the ISIN's"
    );
    registry
        .merge(IsinEntry::new(isin(NOVARTIS)).with_miccode(mic("XOFF")))
        .unwrap();
    assert_eq!(
        currency(&registry, NOVARTIS),
        None,
        "XOFF is of no single country"
    );
    // A stated currency stands; a later statement replaces a default.
    registry
        .merge(
            IsinEntry::new(isin(HOLCIM))
                .with_miccode(mic("XLON"))
                .with_currency(ccy("USD")),
        )
        .unwrap();
    assert_eq!(currency(&registry, HOLCIM).as_deref(), Some("USD"));
    registry
        .merge(IsinEntry::new(isin(SAP)).with_miccode(mic("XETR")))
        .unwrap();
    assert_eq!(currency(&registry, SAP).as_deref(), Some("EUR"));
    assert!(
        registry
            .merge(IsinEntry::new(isin(SAP)).with_currency(ccy("USD")))
            .unwrap()
    );
    assert_eq!(currency(&registry, SAP).as_deref(), Some("USD"));
    // A listing on another market derives that market's.
    assert!(
        registry
            .merge(
                IsinEntry::new(isin(HOLCIM))
                    .with_miccode(mic("XSWX"))
                    .with_ticker(Some(SmolStr::new("HOLN")))
            )
            .unwrap()
    );
    let listed = |registry: &IsinRegistry, market: &str| {
        registry
            .get_listing(HOLCIM, &mic(market).unwrap())
            .and_then(IsinEntry::currency)
            .map(|code| code.as_str().to_owned())
    };
    assert_eq!(listed(&registry, "XSWX").as_deref(), Some("CHF"));
    assert_eq!(listed(&registry, "XLON").as_deref(), Some("USD"));
    // A row read back states what its store held - nothing derived travels
    // with a statement it was not part of: an entry merged over another
    // listing moves only the facts it states.
    let mut other = IsinRegistry::new();
    other
        .merge(entry(HOLCIM, Some(1), &[(IdType::Common, "C-1")]))
        .unwrap();
    assert_eq!(currency(&other, HOLCIM), None);
    assert!(registry.merge(other.get(HOLCIM).unwrap().clone()).unwrap());
    assert_eq!(
        (
            listed(&registry, "XLON").as_deref(),
            listed(&registry, "XSWX").as_deref()
        ),
        (Some("USD"), Some("CHF")),
        "a row of no market states no currency to replace either listing's"
    );

    // An element naming the ticker on the listing takes the ISIN, the
    // SEDOL it embeds and the default currency.
    let mut ticked = OrderEvent::at(20);
    ticked.set_ticker(Some(SmolStr::new("DGE")), true);
    ticked.set_miccode(mic("XLON"), true);
    assert!(registry.fill(&mut ticked));
    assert_eq!(ticked.get_isincode(), Some(DIAGEO));
    assert_eq!(
        ticked.get_securityids().get(&IdType::Sedol),
        Some("0237400")
    );
    assert_eq!(ticked.get_currency().as_str(), "GBP");
}

/// The seed is clean and bound to no store; a write moves the registry it
/// is made on alone, and an empty registry holds none of it.
#[test]
fn a_seeded_registry_is_clean_and_unbound_and_new_holds_none_of_it() {
    crate::install::installed();
    let seeded = IsinRegistry::seeded();
    assert!(!seeded.is_empty());
    assert!(!seeded.is_dirty());
    assert!(seeded.holder().is_none());
    assert_eq!(
        seeded.max_instruments(),
        IsinRegistry::DEFAULT_MAX_INSTRUMENTS
    );
    assert!(IsinRegistry::new().is_empty());
    let mut moved = IsinRegistry::seeded();
    assert!(
        moved
            .merge(entry(APPLE, Some(1), &[(IdType::Common, "C-1")]))
            .unwrap()
    );
    assert!(moved.is_dirty());
    assert_eq!(
        IsinRegistry::seeded()
            .get(APPLE)
            .unwrap()
            .get(&IdType::Common),
        None
    );
}

/// One instrument stated on two markets holds two listing rows, in MIC
/// order: each listing's ticker, currency and listing codes on its own row,
/// the instrument's facts and stamps on both; `get` answers the first, the
/// ticker index leads to each listing on its market, a fill takes the
/// listing of the element's market, and the snapshot streams every row.
#[test]
fn an_instrument_learned_on_two_markets_holds_one_listing_on_each() {
    crate::install::installed();
    let stated = |unix: i64, market: &str, ticker: &str, ric: &str, currency: &str| {
        let mut event = order(
            unix,
            &[
                (IdType::Isin, HOLCIM),
                (IdType::Ric, ric),
                (IdType::Common, "C-1"),
            ],
        );
        event.set_miccode(mic(market), true);
        event.set_ticker(Some(SmolStr::new(ticker)), true);
        event.set_currency(Ccy::new(currency).unwrap(), true);
        event.set_cficode(cfi("ESVUFR"), true);
        event
    };
    let mut registry = IsinRegistry::new();
    assert!(registry.learn(&stated(10, "XSWX", "HOLN", "HOLN.S", "CHF")));
    assert!(registry.learn(&stated(20, "XLON", "0QKY", "HOLN.L", "GBP")));
    assert_eq!((registry.len(), registry.rows()), (1, 2));
    assert_eq!(registry.iter().count(), 2, "every listing row");
    let [london, swiss] = registry.listings(HOLCIM) else {
        panic!("two listings")
    };
    assert_eq!(
        registry.get(HOLCIM),
        Some(london),
        "the first listing: XLON sorts before XSWX"
    );
    assert_eq!(london.miccode().map(Mic::as_str), Some("XLON"));
    assert_eq!(
        (
            london.ticker(),
            london.get(&IdType::Ric),
            london.currency().map(Ccy::as_str)
        ),
        (Some("0QKY"), Some("HOLN.L"), Some("GBP"))
    );
    assert_eq!(
        (
            swiss.ticker(),
            swiss.get(&IdType::Ric),
            swiss.currency().map(Ccy::as_str)
        ),
        (Some("HOLN"), Some("HOLN.S"), Some("CHF"))
    );
    for row in [london, swiss] {
        assert_eq!(row.cficode().map(|code| code.as_str()), Some("ESVUFR"));
        assert_eq!(row.get(&IdType::Common), Some("C-1"));
        assert_eq!(row.get(&IdType::Valor), Some("1221405"));
        assert_eq!(
            (row.updunix(), row.lastunix()),
            (Some(20), Some(20)),
            "the instrument's stamps"
        );
    }
    assert_eq!(
        registry.get_listing(HOLCIM, &mic("XSWX").unwrap()),
        Some(swiss)
    );
    assert_eq!(registry.get_listing(HOLCIM, &mic("XPAR").unwrap()), None);
    assert!(registry.listings(APPLE).is_empty());

    // The ticker index leads to each listing on its market.
    assert_eq!(
        registry.get_by_ticker("HOLN", mic("XSWX").as_ref()),
        Some(swiss)
    );
    assert_eq!(registry.get_by_ticker("0QKY", None), Some(london));
    assert_eq!(
        registry.get_by_ticker("HOLN", mic("XLON").as_ref()),
        None,
        "listed on XSWX alone"
    );

    // A fill takes the listing of the element's market; an element of no
    // market, or of a market the instrument is not listed on, takes the
    // instrument's facts alone.
    let mut on_swiss = order(30, &[(IdType::Isin, HOLCIM)]);
    on_swiss.set_miccode(mic("XSWX"), true);
    assert!(registry.fill(&mut on_swiss));
    assert_eq!(on_swiss.get_securityids().get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(on_swiss.get_ticker(), Some("HOLN"));
    assert_eq!(on_swiss.get_currency().as_str(), "CHF");
    for market in [None, mic("XPAR")] {
        let mut element = order(30, &[(IdType::Isin, HOLCIM)]);
        element.set_miccode(market.clone(), true);
        assert!(registry.fill(&mut element), "{market:?}");
        let ids = element.get_securityids();
        assert_eq!(ids.get(&IdType::Common), Some("C-1"), "{market:?}");
        assert_eq!(ids.get(&IdType::Ric), None, "{market:?}");
        assert_eq!(element.get_ticker(), None, "{market:?}");
        assert!(element.get_currency().is_none(), "{market:?}");
        assert_eq!(
            element.get_cficode().map(|code| code.as_str()),
            Some("ESVUFR"),
            "{market:?}"
        );
    }

    // The snapshot streams every listing row, in ISIN then MIC order, and
    // reads back as the same listings.
    let back = IsinRegistry::from_arrow_reader(registry.into_arrow_reader().unwrap()).unwrap();
    assert!(back.iter().eq(registry.iter()));
    assert_eq!((back.len(), back.rows()), (1, 2));

    // One ticker on both markets names each listing on its own, and the
    // instrument where no market is stated: the index holds one slot per
    // ISIN since local codes became keys, so two listings of one ISIN are
    // one answer - its first listing, its listing facts withheld - where
    // two (ISIN, market) slots answered none before.
    assert!(registry.learn(&stated(40, "XLON", "HOLN", "HOLN.L", "GBP")));
    for market in ["XLON", "XSWX"] {
        assert_eq!(
            registry
                .get_by_ticker("HOLN", mic(market).as_ref())
                .and_then(IsinEntry::miccode),
            mic(market).as_ref()
        );
    }
    assert_eq!(
        registry.get_by_ticker("HOLN", None),
        registry.get(HOLCIM),
        "one instrument, its first listing"
    );
    let mut unmarketed = OrderEvent::at(50);
    unmarketed.set_ticker(Some(SmolStr::new("HOLN")), true);
    let Resolution::Matched {
        entry: found,
        tier: MatchTier::Symbology,
        derived: true,
        listing: false,
    } = registry.resolve(&unmarketed)
    else {
        panic!("one instrument, its listing facts withheld")
    };
    assert_eq!(found.isin().as_str(), HOLCIM);
    assert!(registry.get_by_ticker("0QKY", None).is_none(), "renamed");
}

/// A statement naming no market lands its listing facts on its ISIN's
/// single row, and on none where the ISIN has several - its instrument
/// facts on every one.
#[test]
fn a_statement_of_no_market_lands_on_a_single_listing_and_none_of_several() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    registry
        .merge(entry(HOLCIM, Some(1), &[]).with_miccode(mic("XSWX")))
        .unwrap();
    assert!(
        registry
            .merge(
                entry(HOLCIM, Some(2), &[(IdType::Ric, "HOLN.S")])
                    .with_ticker(Some(SmolStr::new("HOLN")))
            )
            .unwrap()
    );
    let swiss = registry.get(HOLCIM).unwrap();
    assert_eq!(
        (swiss.get(&IdType::Ric), swiss.ticker()),
        (Some("HOLN.S"), Some("HOLN")),
        "the single listing takes them"
    );
    registry
        .merge(entry(HOLCIM, Some(3), &[]).with_miccode(mic("XLON")))
        .unwrap();
    assert!(
        !registry
            .merge(
                entry(HOLCIM, Some(4), &[(IdType::Ric, "HOLN.X")])
                    .with_ticker(Some(SmolStr::new("HOLX")))
                    .with_currency(ccy("EUR"))
            )
            .unwrap(),
        "of two listings, it lands on none"
    );
    assert!(registry.get_by_ticker("HOLX", None).is_none());
    assert!(
        registry
            .merge(entry(
                HOLCIM,
                Some(5),
                &[(IdType::Ric, "HOLN.X"), (IdType::Common, "C-1")]
            ))
            .unwrap()
    );
    let mut unmarked = order(6, &[(IdType::Isin, HOLCIM), (IdType::Belgian, "B-1")]);
    unmarked.set_ticker(Some(SmolStr::new("HOLY")), true);
    assert!(registry.learn(&unmarked));
    for row in registry.listings(HOLCIM) {
        let market = row.miccode();
        assert_eq!(row.get(&IdType::Common), Some("C-1"), "{market:?}");
        assert_eq!(row.get(&IdType::Belgian), Some("B-1"), "{market:?}");
        assert_ne!(row.get(&IdType::Ric), Some("HOLN.X"), "{market:?}");
        assert_ne!(row.currency().map(Ccy::as_str), Some("EUR"), "{market:?}");
        assert_ne!(row.ticker(), Some("HOLY"), "{market:?}");
        assert_eq!((row.updunix(), row.lastunix()), (Some(6), Some(6)));
    }
    assert!(registry.get_by_ticker("HOLY", None).is_none());
}

/// While no market is known an ISIN holds its unlisted row alone, and the
/// first listing takes it over - its market filled, nothing duplicated - a
/// ticker it stated leading to any market until then and to that listing's
/// alone after; a second market is a second listing.
#[test]
fn the_unlisted_row_is_taken_over_by_the_first_listing() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    registry
        .merge(
            entry(DIAGEO, Some(1), &[(IdType::Common, "C-1")])
                .with_ticker(Some(SmolStr::new("DGE"))),
        )
        .unwrap();
    let unlisted = registry.get(DIAGEO).unwrap();
    assert_eq!((unlisted.miccode(), unlisted.currency()), (None, None));
    assert!(
        registry
            .get_by_ticker("DGE", mic("XPAR").as_ref())
            .is_some(),
        "a row listed on no market answers any"
    );
    assert!(
        registry
            .merge(IsinEntry::new(isin(DIAGEO)).with_miccode(mic("XLON")))
            .unwrap()
    );
    assert_eq!(registry.rows(), 1, "taken over, nothing duplicated");
    let listed = registry.get(DIAGEO).unwrap();
    assert_eq!(
        (
            listed.miccode().map(Mic::as_str),
            listed.ticker(),
            listed.currency().map(Ccy::as_str)
        ),
        (Some("XLON"), Some("DGE"), Some("GBP"))
    );
    assert_eq!(listed.get(&IdType::Common), Some("C-1"));
    assert!(
        registry
            .get_by_ticker("DGE", mic("XPAR").as_ref())
            .is_none()
    );
    assert_eq!(
        registry
            .get_by_ticker("DGE", mic("XLON").as_ref())
            .and_then(IsinEntry::miccode),
        mic("XLON").as_ref()
    );
    assert!(
        registry
            .merge(
                IsinEntry::new(isin(DIAGEO))
                    .with_miccode(mic("XNYS"))
                    .with_ticker(Some(SmolStr::new("DEO")))
            )
            .unwrap()
    );
    assert_eq!(registry.rows(), 2);
    assert!(
        registry
            .listings(DIAGEO)
            .iter()
            .all(|row| row.miccode().is_some())
    );
    assert_eq!(
        registry
            .get_by_ticker("DEO", None)
            .and_then(IsinEntry::miccode),
        mic("XNYS").as_ref()
    );
}

/// `remove_listing` removes one listing row, its ticker with it, and the
/// instrument with its last; `remove` answers every listing in MIC order.
#[test]
fn a_listing_is_removed_alone_and_the_instrument_with_its_last() {
    crate::install::installed();
    let two = || {
        let mut registry = IsinRegistry::new();
        for (market, ticker) in [("XSWX", "HOLN"), ("XLON", "0QKY")] {
            registry
                .merge(
                    entry(HOLCIM, Some(1), &[(IdType::Common, "C-1")])
                        .with_miccode(mic(market))
                        .with_ticker(Some(SmolStr::new(ticker))),
                )
                .unwrap();
        }
        registry
    };
    let mut registry = two();
    assert_eq!(registry.remove_listing(HOLCIM, &mic("XPAR").unwrap()), None);
    let removed = registry
        .remove_listing(HOLCIM, &mic("XLON").unwrap())
        .unwrap();
    assert_eq!(removed.ticker(), Some("0QKY"));
    assert_eq!((registry.len(), registry.rows()), (1, 1));
    assert!(registry.get_by_ticker("0QKY", None).is_none());
    assert_eq!(
        registry.get(HOLCIM).and_then(IsinEntry::ticker),
        Some("HOLN")
    );
    assert!(registry.is_dirty());
    assert!(
        registry
            .remove_listing(HOLCIM, &mic("XSWX").unwrap())
            .is_some()
    );
    assert!(registry.is_empty() && registry.get(HOLCIM).is_none());
    assert!(registry.get_by_ticker("HOLN", None).is_none());

    let mut registry = two();
    let removed: Vec<_> = registry
        .remove(HOLCIM)
        .iter()
        .map(|row| row.miccode().map(Mic::as_str).map(str::to_owned))
        .collect();
    assert_eq!(removed, [Some("XLON".into()), Some("XSWX".into())]);
    assert!(registry.is_empty());
    assert!(registry.get_by_ticker("HOLN", None).is_none());
}

/// `lastunix` is the latest instant an event the registry learned an
/// instrument from: every learn moves it - one stating the ISIN alone
/// included, which makes a new instrument's row - so the registry is dirty
/// and the next commit writes it; an earlier event moves nothing, a stated
/// one merges later-wins, every listing holds it, and `updunix` moves only
/// with a fact.
#[test]
fn every_learn_moves_lastunix_and_only_a_fact_moves_updunix() {
    crate::install::installed();
    let met = |unix: i64| order(unix, &[(IdType::Isin, APPLE)]);
    let mut registry = IsinRegistry::new();
    assert!(registry.learn(&met(10)), "an instrument met is learned");
    let row = registry.get(APPLE).unwrap();
    assert_eq!((row.updunix(), row.lastunix()), (Some(10), Some(10)));
    assert_eq!(row.get(&IdType::Cusip), Some("037833100"));

    let mut loaded =
        IsinRegistry::from_arrow_reader(registry.into_arrow_reader().unwrap()).unwrap();
    assert!(!loaded.is_dirty());
    assert_eq!(
        loaded.get(APPLE).unwrap().lastunix(),
        Some(10),
        "the snapshot carries it"
    );
    assert!(loaded.learn(&met(20)), "met later");
    assert!(loaded.is_dirty(), "and committed next");
    let row = loaded.get(APPLE).unwrap();
    assert_eq!(
        (row.updunix(), row.lastunix()),
        (Some(10), Some(20)),
        "no fact moved"
    );

    let mut clean = IsinRegistry::from_arrow_reader(loaded.into_arrow_reader().unwrap()).unwrap();
    assert!(!clean.learn(&met(15)), "met earlier");
    assert!(!clean.learn(&met(20)), "met again at the same instant");
    assert!(!clean.is_dirty());
    let mut classified = met(25);
    classified.set_cficode(cfi("ESVUFR"), true);
    assert!(clean.learn(&classified));
    let row = clean.get(APPLE).unwrap();
    assert_eq!((row.updunix(), row.lastunix()), (Some(25), Some(25)));

    // A stated one merges later-wins, moving no `updunix`.
    assert!(
        !clean
            .merge(IsinEntry::new(isin(APPLE)).with_lastunix(Some(5)))
            .unwrap()
    );
    assert!(
        clean
            .merge(IsinEntry::new(isin(APPLE)).with_lastunix(Some(40)))
            .unwrap()
    );
    let row = clean.get(APPLE).unwrap();
    assert_eq!((row.updunix(), row.lastunix()), (Some(25), Some(40)));

    // Every listing holds it.
    for market in ["XNAS", "XETR"] {
        clean
            .merge(IsinEntry::new(isin(APPLE)).with_miccode(mic(market)))
            .unwrap();
    }
    assert_eq!(clean.rows(), 2);
    assert!(clean.learn(&met(50)));
    for row in clean.listings(APPLE) {
        assert_eq!(row.lastunix(), Some(50), "{:?}", row.miccode());
    }
    // A learn past the bound makes no row.
    let mut bounded = IsinRegistry::new().with_max_instruments(1);
    assert!(bounded.learn(&met(1)));
    assert!(!bounded.learn(&order(1, &[(IdType::Isin, HOLCIM)])));
    assert_eq!(bounded.len(), 1);
}

/// A golden file states one ISIN on several markets as several rows, which
/// load as its listings - the instrument's facts any row states on every
/// one - and `lastunix` under any of its spellings, the later of two kept.
#[test]
fn a_golden_file_with_two_rows_of_one_isin_loads_two_listings() {
    crate::install::installed();
    use arrow_array::TimestampNanosecondArray;
    for name in ["lastunix", "LastSeen", "last_seen_unix"] {
        let schema = Arc::new(Schema::new(vec![
            ArrowField::new("ISIN", ArrowType::Utf8, false),
            ArrowField::new("MIC", ArrowType::Utf8, true),
            ArrowField::new("Ticker", ArrowType::Utf8, true),
            ArrowField::new("Currency", ArrowType::Utf8, true),
            ArrowField::new("CFI", ArrowType::Utf8, true),
            ArrowField::new(
                name,
                ArrowType::Timestamp(arrow_schema::TimeUnit::Nanosecond, Some("UTC".into())),
                true,
            ),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(vec![HSBC, HSBC, APPLE])),
                Arc::new(StringArray::from(vec![
                    Some("XLON"),
                    Some("XHKG"),
                    Some("XNAS"),
                ])),
                Arc::new(StringArray::from(vec![
                    Some("HSBA"),
                    Some("0005"),
                    Some("AAPL"),
                ])),
                Arc::new(StringArray::from(vec![Some("GBP"), Some("HKD"), None])),
                Arc::new(StringArray::from(vec![Some("ESVUFR"), None, None])),
                Arc::new(
                    TimestampNanosecondArray::from(vec![Some(30), Some(20), None])
                        .with_timezone("UTC"),
                ),
            ],
        )
        .unwrap();
        let registry =
            IsinRegistry::from_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch])).unwrap();
        assert_eq!((registry.len(), registry.rows()), (2, 3), "{name}");
        let listings: Vec<_> = registry
            .listings(HSBC)
            .iter()
            .map(|row| {
                (
                    row.miccode().map(Mic::as_str),
                    row.ticker(),
                    row.currency().map(Ccy::as_str),
                )
            })
            .collect();
        assert_eq!(
            listings,
            [
                (Some("XHKG"), Some("0005"), Some("HKD")),
                (Some("XLON"), Some("HSBA"), Some("GBP"))
            ],
            "{name}"
        );
        for row in registry.listings(HSBC) {
            assert_eq!(
                row.cficode().map(|code| code.as_str()),
                Some("ESVUFR"),
                "{name}"
            );
            assert_eq!(row.get(&IdType::Sedol), Some("0540528"), "{name}");
            assert_eq!(row.lastunix(), Some(30), "{name}: the later");
        }
        assert_eq!(registry.get(APPLE).unwrap().lastunix(), None, "{name}");
        assert_eq!(
            registry
                .get_by_ticker("0005", None)
                .and_then(IsinEntry::miccode)
                .map(Mic::as_str),
            Some("XHKG"),
            "{name}"
        );
    }
}

/// The origin currency is an instrument fact held only where stated: a
/// golden file's column under each of its spellings, a merge on any
/// listing - every listing then holds it - and a learn; `XXX` states none,
/// and no fold derives one, from the ISIN's prefix or from a listing's
/// currency. A
/// EUR listing of a USD share class learned from an element stating only
/// its currency leaves the instrument's origin USD; the fill lands the
/// origin into an element holding none and never over one it holds; an FX
/// pair's origin is never learned.
#[test]
fn the_origin_currency_is_an_instrument_fact_stated_and_never_derived() {
    crate::install::installed();
    const CSPX: &str = "IE00B5BMR087";
    let origin = |registry: &IsinRegistry, key: &str| {
        registry
            .get(key)
            .and_then(IsinEntry::origccy)
            .map(|code| code.as_str().to_owned())
    };
    for name in [
        "origccy",
        "OrigCurrency",
        "OriginalCurrency",
        "original_currency",
        "Issue_Currency",
    ] {
        let golden = IsinRegistry::from_arrow_reader(flat(&[
            ("isin", vec![Some(CSPX), Some(HOLCIM)]),
            ("mic", vec![Some("XLON"), Some("XSWX")]),
            ("currency", vec![Some("GBP"), Some("CHF")]),
            (name, vec![Some("USD"), None]),
        ]))
        .unwrap();
        assert_eq!(origin(&golden, CSPX).as_deref(), Some("USD"), "{name}");
        assert_eq!(origin(&golden, HOLCIM), None, "{name}: none from CHF");
    }

    let mut registry = IsinRegistry::new();
    registry
        .merge(
            IsinEntry::new(isin(CSPX))
                .with_miccode(mic("XLON"))
                .with_currency(ccy("USD")),
        )
        .unwrap();
    assert_eq!(
        origin(&registry, CSPX),
        None,
        "neither the prefix's EUR nor the listing's USD is derived"
    );
    assert_eq!(
        IsinEntry::new(isin(CSPX))
            .with_origccy(ccy("XXX"))
            .origccy(),
        None,
        "XXX states none"
    );
    assert!(
        registry
            .merge(
                IsinEntry::new(isin(CSPX))
                    .with_miccode(mic("XETR"))
                    .with_currency(ccy("EUR"))
                    .with_origccy(ccy("USD")),
            )
            .unwrap()
    );
    assert_eq!(registry.rows(), 2);
    for row in registry.listings(CSPX) {
        assert_eq!(
            row.origccy().map(|code| code.as_str()),
            Some("USD"),
            "{:?}: every listing",
            row.miccode()
        );
    }
    assert!(
        !registry
            .merge(IsinEntry::new(isin(CSPX)).with_origccy(ccy("USD")))
            .unwrap(),
        "restated, it moves nothing"
    );
    let xetra = registry
        .get_listing(CSPX, &Mic::new("XETR").unwrap())
        .unwrap()
        .clone();
    assert_eq!(IsinEntry::from_scalar(&xetra.into_scalar()).unwrap(), xetra);

    // A EUR listing of the USD class, learned from an order stating only
    // its currency: the listing's currency is EUR, the origin stays USD.
    let mut seeded = IsinRegistry::seeded();
    assert_eq!(origin(&seeded, CSPX).as_deref(), Some("USD"));
    let mut sxr8 = order(10, &[(IdType::Isin, CSPX)]);
    sxr8.set_miccode(mic("XETR"), true);
    sxr8.set_ticker(Some(SmolStr::new("SXR8")), true);
    sxr8.set_currency(Ccy::new("EUR").unwrap(), true);
    assert!(sxr8.get_origccy().is_none());
    assert!(seeded.learn(&sxr8));
    let listing = seeded
        .get_listing(CSPX, &Mic::new("XETR").unwrap())
        .unwrap();
    assert_eq!(listing.currency().map(|code| code.as_str()), Some("EUR"));
    for row in seeded.listings(CSPX) {
        assert_eq!(
            row.origccy().map(|code| code.as_str()),
            Some("USD"),
            "{:?}: never EUR",
            row.miccode()
        );
    }
    // The fill lands the origin where the element holds none...
    let mut filled = order(20, &[(IdType::Isin, CSPX)]);
    filled.set_miccode(mic("XETR"), true);
    assert!(seeded.fill(&mut filled));
    assert_eq!(filled.get_origccy().as_str(), "USD");
    assert_eq!(filled.get_currency().as_str(), "EUR");
    assert_eq!(filled.origin_currency().as_str(), "USD");
    // ...and never over one it holds.
    let mut held = order(20, &[(IdType::Isin, CSPX)]);
    held.set_origccy(Ccy::new("GBP").unwrap(), true);
    seeded.fill(&mut held);
    assert_eq!(held.get_origccy().as_str(), "GBP");
    // An instrument stating none fills none: the element reads its own
    // currency as its origin.
    let mut holcim = order(20, &[(IdType::Isin, HOLCIM)]);
    holcim.set_miccode(mic("XSWX"), true);
    assert!(seeded.fill(&mut holcim));
    assert!(holcim.get_origccy().is_none());
    assert_eq!(holcim.origin_currency().as_str(), "CHF");

    // A stated origin is learned; an FX pair's never is.
    let mut learned = IsinRegistry::new();
    let mut stated = order(5, &[(IdType::Isin, APPLE)]);
    stated.set_origccy(Ccy::new("USD").unwrap(), true);
    assert!(learned.learn(&stated));
    assert_eq!(origin(&learned, APPLE).as_deref(), Some("USD"));
    let fx = numbered("EZ");
    let mut traded = order(
        10,
        &[(IdType::Isin, fx.as_str()), (IdType::Forex, "EUR/USD")],
    );
    traded.set_currency(Ccy::new("EUR").unwrap(), true);
    traded.set_origccy(Ccy::new("EUR").unwrap(), true);
    assert!(learned.learn(&traded));
    assert_eq!(origin(&learned, &fx), None, "a pair's is no instrument's");
}

/// `firstunix` is the earliest instant an event the registry learned an
/// instrument from: the first learn sets it beside `lastunix`, every
/// listing holds it, an earlier event moves it back - the registry dirty -
/// and a later one never moves it, so a learn moving neither stamp nor a
/// fact leaves the registry clean; a stated one merges earlier-wins; a
/// golden file states it under each of its spellings; the snapshot carries
/// it.
#[test]
fn the_first_learn_sets_firstunix_and_only_an_earlier_event_moves_it() {
    crate::install::installed();
    let met = |unix: i64| order(unix, &[(IdType::Isin, APPLE)]);
    let mut registry = IsinRegistry::new();
    assert!(registry.learn(&met(20)));
    let row = registry.get(APPLE).unwrap();
    assert_eq!(
        (row.updunix(), row.firstunix(), row.lastunix()),
        (Some(20), Some(20), Some(20))
    );
    let mut loaded =
        IsinRegistry::from_arrow_reader(registry.into_arrow_reader().unwrap()).unwrap();
    assert_eq!(loaded.get(APPLE).unwrap().firstunix(), Some(20), "carried");
    assert!(!loaded.learn(&met(20)), "met again at the same instant");
    assert!(loaded.learn(&met(30)), "a later event moves lastunix");
    assert_eq!(
        loaded.get(APPLE).unwrap().firstunix(),
        Some(20),
        "not firstunix"
    );
    let mut clean = IsinRegistry::from_arrow_reader(loaded.into_arrow_reader().unwrap()).unwrap();
    assert!(!clean.learn(&met(25)), "between the two: nothing moves");
    assert!(!clean.is_dirty());
    assert!(clean.learn(&met(5)), "replayed out of order");
    assert!(clean.is_dirty());
    let row = clean.get(APPLE).unwrap();
    assert_eq!(
        (row.updunix(), row.firstunix(), row.lastunix()),
        (Some(20), Some(5), Some(30)),
        "updunix moves with a fact alone"
    );
    // A stated one merges earlier-wins, and every listing holds it.
    assert!(
        !clean
            .merge(IsinEntry::new(isin(APPLE)).with_firstunix(Some(9)))
            .unwrap()
    );
    assert!(
        clean
            .merge(IsinEntry::new(isin(APPLE)).with_firstunix(Some(1)))
            .unwrap()
    );
    for market in ["XNAS", "XETR"] {
        clean
            .merge(IsinEntry::new(isin(APPLE)).with_miccode(mic(market)))
            .unwrap();
    }
    assert_eq!(clean.rows(), 2);
    for row in clean.listings(APPLE) {
        assert_eq!(row.firstunix(), Some(1), "{:?}", row.miccode());
    }
    // A golden file states it under each of its spellings.
    for name in ["firstunix", "FirstSeen", "first_seen_unix"] {
        use arrow_array::TimestampNanosecondArray;
        let schema = Arc::new(Schema::new(vec![
            ArrowField::new("ISIN", ArrowType::Utf8, false),
            ArrowField::new(
                name,
                ArrowType::Timestamp(arrow_schema::TimeUnit::Nanosecond, Some("UTC".into())),
                true,
            ),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(vec![APPLE, APPLE])),
                Arc::new(
                    TimestampNanosecondArray::from(vec![Some(30), Some(20)]).with_timezone("UTC"),
                ),
            ],
        )
        .unwrap();
        let golden =
            IsinRegistry::from_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch])).unwrap();
        assert_eq!(
            golden.get(APPLE).unwrap().firstunix(),
            Some(20),
            "{name}: the earlier"
        );
    }
}

/// The exact tier, in its order: a real ISIN decides alone over a code
/// naming another instrument, a code over a ticker naming another, the
/// ticker on its market last - each answering its tier and whether the ISIN
/// was derived and the row's listing facts are the element's.
#[test]
fn the_cascade_takes_the_isin_then_a_code_then_the_ticker_on_its_market() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    registry
        .merge(
            IsinEntry::new(isin(APPLE))
                .with_miccode(mic("XNAS"))
                .with_ticker(Some(SmolStr::new("AAPL"))),
        )
        .unwrap();
    registry
        .merge(
            IsinEntry::new(isin(DIAGEO))
                .with_miccode(mic("XLON"))
                .with_ticker(Some(SmolStr::new("DGE"))),
        )
        .unwrap();
    registry
        .merge(IsinEntry::new(isin(SAP)).with_miccode(mic("XETR")))
        .unwrap();
    // The ISIN wins over a CUSIP naming Apple.
    let stated = order(1, &[(IdType::Isin, SAP), (IdType::Cusip, "037833100")]);
    let Resolution::Matched {
        entry: row,
        tier: MatchTier::Isin,
        derived: false,
        listing: true,
    } = registry.resolve(&stated)
    else {
        panic!("{:?}", registry.resolve(&stated))
    };
    assert_eq!(row.isin().as_str(), SAP);
    // A CUSIP wins over a ticker naming Diageo.
    let mut coded = order(1, &[(IdType::Cusip, "037833100")]);
    coded.set_ticker(Some(SmolStr::new("DGE")), true);
    coded.set_miccode(mic("XLON"), true);
    let Resolution::Matched {
        entry: row,
        tier: MatchTier::Code(IdType::Cusip),
        derived: true,
        listing: false,
    } = registry.resolve(&coded)
    else {
        panic!("{:?}", registry.resolve(&coded))
    };
    assert_eq!(row.isin().as_str(), APPLE, "Apple is not listed on XLON");
    // The ticker on its market last.
    let mut ticked = OrderEvent::at(1);
    ticked.set_ticker(Some(SmolStr::new("DGE")), true);
    ticked.set_miccode(mic("XLON"), true);
    let Resolution::Matched {
        entry: row,
        tier: MatchTier::Symbology,
        derived: true,
        listing: true,
    } = registry.resolve(&ticked)
    else {
        panic!("{:?}", registry.resolve(&ticked))
    };
    assert_eq!(row.isin().as_str(), DIAGEO);
    // The same through the public lookups.
    assert_eq!(
        registry
            .get_by_code(&IdType::Cusip, "037833100", None)
            .map(|row| row.isin().as_str()),
        Some(APPLE)
    );
    assert_eq!(
        registry
            .get_by_code(&IdType::Sedol, "0237400", mic("XLON").as_ref())
            .map(|row| row.isin().as_str()),
        Some(DIAGEO),
        "the SEDOL its ISIN embeds"
    );
    assert_eq!(
        registry.get_by_code(&IdType::Cusip, "not a cusip", None),
        None
    );
    assert_eq!(registry.get_by_code(&IdType::IsoCcy, "USD", None), None);
    // Nothing stated: no key.
    assert_eq!(
        registry.resolve(&OrderEvent::at(1)),
        Resolution::Unmatched(Unmatched::NoKey)
    );
    let mut unknown = OrderEvent::at(1);
    unknown.set_ticker(Some(SmolStr::new("ZZZZ")), true);
    assert_eq!(
        registry.resolve(&unknown),
        Resolution::Unmatched(Unmatched::NoCandidate)
    );
}

/// A real ISIN the registry lacks ends the cascade: a CUSIP beside it that
/// another row holds names nothing, and a fill takes nothing - another
/// instrument's codes would surround the one it states.
#[test]
fn an_unknown_stated_isin_ends_the_cascade_and_fills_nothing() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    registry
        .merge(IsinEntry::new(isin(APPLE)).with_miccode(mic("XNAS")))
        .unwrap();
    let mut stated = order(1, &[(IdType::Isin, NOVARTIS), (IdType::Cusip, "037833100")]);
    assert_eq!(
        registry.resolve(&stated),
        Resolution::Unmatched(Unmatched::UnknownIsin {
            stated: isin(NOVARTIS)
        })
    );
    assert!(!registry.fill(&mut stated));
    assert_eq!(stated.get_isincode(), Some(NOVARTIS));
    assert_eq!(stated.get_cficode(), None);
}

/// A code two instruments hold is ambiguous: the cascade stops there - the
/// ticker beside it naming one instrument is not read, nor is any short
/// name scanned - and a fill takes nothing.
#[test]
fn a_code_two_instruments_hold_is_ambiguous_and_stops_the_cascade() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    for text in [APPLE, SAP] {
        registry
            .merge(entry(text, Some(1), &[(IdType::Common, "C-1")]))
            .unwrap();
    }
    registry
        .merge(
            IsinEntry::new(isin(DIAGEO))
                .with_miccode(mic("XLON"))
                .with_ticker(Some(SmolStr::new("DGE"))),
        )
        .unwrap();
    let mut stated = order(1, &[(IdType::Common, "C-1")]);
    stated.set_ticker(Some(SmolStr::new("DGE")), true);
    stated.set_miccode(mic("XLON"), true);
    assert_eq!(
        registry.resolve(&stated),
        Resolution::Unmatched(Unmatched::Ambiguous {
            tier: MatchTier::Code(IdType::Common),
            isins: vec![isin(SAP), isin(APPLE)],
        })
    );
    assert!(!registry.fill(&mut stated));
    assert_eq!(stated.get_isincode(), None);
}

/// One instrument on two markets is one answer, never an ambiguity: the
/// seed's HSBC SEDOL with no market resolves to its ISIN, and an instrument
/// listed on two markets in one currency is matched by its CUSIP and by its
/// short name alike. An element on a market the instrument is not listed on
/// takes its instrument facts and none of the listing's: no RIC, no ticker.
#[test]
fn the_listings_of_one_instrument_are_one_match() {
    crate::install::installed();
    let seeded = IsinRegistry::seeded();
    let sedol = order(1, &[(IdType::Sedol, "0540528")]);
    let Resolution::Matched {
        entry: row,
        tier: MatchTier::Code(IdType::Sedol),
        derived: true,
        ..
    } = seeded.resolve(&sedol)
    else {
        panic!("{:?}", seeded.resolve(&sedol))
    };
    assert_eq!(row.isin().as_str(), HSBC);
    assert_eq!(seeded.listings(HSBC).len(), 2);

    let mut registry = IsinRegistry::new();
    for market in ["XNAS", "XNYS"] {
        registry
            .merge(
                IsinEntry::new(isin(APPLE))
                    .with_miccode(mic(market))
                    .with_fisn(fisn("APPLE INC/SH")),
            )
            .unwrap();
    }
    assert_eq!(registry.rows(), 2);
    let by_cusip = order(1, &[(IdType::Cusip, "037833100")]);
    assert!(matches!(
        registry.resolve(&by_cusip),
        Resolution::Matched {
            tier: MatchTier::Code(IdType::Cusip),
            ..
        }
    ));
    let mut by_name = order(1, &[(IdType::Fisn, "APPLE INC/SH")]);
    by_name.set_currency(Ccy::new("USD").unwrap(), true);
    let Resolution::Matched {
        entry: row,
        tier: MatchTier::Economic { similarity },
        derived: true,
        ..
    } = registry.resolve(&by_name)
    else {
        panic!("{:?}", registry.resolve(&by_name))
    };
    assert_eq!((row.isin().as_str(), similarity), (APPLE, 1.0));

    let mut london = IsinRegistry::new();
    london
        .merge(
            entry(APPLE, Some(1), &[(IdType::Ric, "AAPL.L")])
                .with_miccode(mic("XLON"))
                .with_ticker(Some(SmolStr::new("0R2V"))),
        )
        .unwrap();
    let mut swiss = order(2, &[(IdType::Cusip, "037833100")]);
    swiss.set_miccode(mic("XSWX"), true);
    assert!(matches!(
        london.resolve(&swiss),
        Resolution::Matched {
            tier: MatchTier::Code(IdType::Cusip),
            derived: true,
            listing: false,
            ..
        }
    ));
    assert!(london.fill(&mut swiss));
    assert_eq!(swiss.get_isincode(), Some(APPLE));
    assert_eq!(swiss.get_securityids().get(&IdType::Ric), None);
    assert_eq!(swiss.get_ticker(), None);
}

/// The economic tier: the instrument listed in the element's currency whose
/// short name is the most similar, at least the threshold - `APPLE INC/SH`
/// against `APPLE INC./SH`, one insertion in thirteen - while a name four
/// bytes longer is below it by its length alone; another stated CFI
/// category or origin currency drops the instrument and names the
/// conflict, an unclassified `XXXXXX` conflicts with nothing, another
/// currency is never a candidate, two equal bests are ambiguous, and no
/// short name or a currency of `XXX` is no candidate, never a scan.
#[test]
fn the_economic_tier_matches_a_similar_short_name_in_the_same_currency() {
    crate::install::installed();
    let usd = || Ccy::new("USD").unwrap();
    let named = |name: &str| {
        let mut element = order(1, &[(IdType::Fisn, name)]);
        element.set_currency(usd(), true);
        element
    };
    let registry_of = |name: &str, code: &str| {
        let mut registry = IsinRegistry::new();
        registry
            .merge(
                IsinEntry::new(isin(APPLE))
                    .with_miccode(mic("XNAS"))
                    .with_fisn(fisn(name))
                    .with_cficode(cfi(code)),
            )
            .unwrap();
        registry
    };
    let registry = registry_of("APPLE INC./SH", "ESVUFR");
    let Resolution::Matched {
        entry: row,
        tier: MatchTier::Economic { similarity },
        derived: true,
        listing: true,
    } = registry.resolve(&named("APPLE INC/SH"))
    else {
        panic!("{:?}", registry.resolve(&named("APPLE INC/SH")))
    };
    assert_eq!(row.isin().as_str(), APPLE);
    assert!((similarity - 12.0 / 13.0).abs() < 1e-12, "{similarity}");
    assert!(similarity >= IsinRegistry::DEFAULT_ECONOMIC_THRESHOLD);

    let plain = registry_of("APPLE INC/SH", "ESVUFR");
    assert_eq!(
        plain.resolve(&named("APPLE INC/SH USD")),
        Resolution::Unmatched(Unmatched::BelowThreshold {
            best: 0.75,
            isin: isin(APPLE)
        })
    );

    let mut equity = named("APPLE INC/SH");
    equity.set_cficode(cfi("ESVUFR"), true);
    assert_eq!(
        registry_of("APPLE INC/SH", "DBFTFR").resolve(&equity),
        Resolution::Unmatched(Unmatched::CfiConflict {
            stated: 'E',
            held: 'D',
            isin: isin(APPLE)
        })
    );
    let mut unclassified = named("APPLE INC/SH");
    unclassified.set_cficode(Some(Cfi::new("XXXXXX").unwrap()), true);
    assert!(matches!(
        plain.resolve(&unclassified),
        Resolution::Matched {
            tier: MatchTier::Economic { .. },
            ..
        }
    ));

    let mut euro = order(1, &[(IdType::Fisn, "APPLE INC/SH")]);
    euro.set_currency(Ccy::new("EUR").unwrap(), true);
    assert_eq!(
        plain.resolve(&euro),
        Resolution::Unmatched(Unmatched::NoCandidate),
        "another currency is no candidate"
    );

    let mut issued = IsinRegistry::new();
    issued
        .merge(
            IsinEntry::new(isin(APPLE))
                .with_miccode(mic("XNAS"))
                .with_fisn(fisn("APPLE INC/SH"))
                .with_origccy(ccy("USD")),
        )
        .unwrap();
    let mut other_origin = named("APPLE INC/SH");
    other_origin.set_origccy(Ccy::new("EUR").unwrap(), true);
    assert_eq!(
        issued.resolve(&other_origin),
        Resolution::Unmatched(Unmatched::CurrencyConflict {
            stated: Ccy::new("EUR").unwrap(),
            held: usd(),
            isin: isin(APPLE)
        })
    );

    let mut twins = registry_of("APPLE INC/SH", "ESVUFR");
    let other = numbered("US");
    twins
        .merge(
            IsinEntry::new(isin(&other))
                .with_miccode(mic("XNYS"))
                .with_fisn(fisn("APPLE INC/SH")),
        )
        .unwrap();
    assert_eq!(
        twins.resolve(&named("APPLE INC/SH")),
        Resolution::Unmatched(Unmatched::Ambiguous {
            tier: MatchTier::Economic { similarity: 1.0 },
            isins: vec![isin(&other), isin(APPLE)],
        })
    );

    let mut nameless = OrderEvent::at(1);
    nameless.set_ticker(Some(SmolStr::new("ZZZZ")), true);
    nameless.set_currency(usd(), true);
    assert_eq!(
        plain.resolve(&nameless),
        Resolution::Unmatched(Unmatched::NoCandidate)
    );
    let mut unpriced = order(1, &[(IdType::Fisn, "APPLE INC/SH")]);
    unpriced.set_currency(Ccy::new("XXX").unwrap(), true);
    assert_eq!(
        plain.resolve(&unpriced),
        Resolution::Unmatched(Unmatched::NoCandidate),
        "XXX states no currency"
    );
}

/// The economic match is weighed by `resolve` always and taken by a fill
/// only where the registry says so - `false` by default - landing the ISIN
/// as a derivation; the threshold is a similarity in `(0, 1]`, and a value
/// outside it or NaN is refused by value. A parse door's fill never takes
/// it ([`yggdryl_fix::FixCodec`]'s lifecycle is pinned in `rust/fix/tests/root/enrich.rs`).
#[test]
fn a_fill_takes_the_economic_match_only_where_it_is_enabled() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    registry
        .merge(
            IsinEntry::new(isin(APPLE))
                .with_miccode(mic("XNAS"))
                .with_fisn(fisn("APPLE INC./SH")),
        )
        .unwrap();
    assert!(!registry.is_economic_match());
    assert_eq!(
        registry.economic_threshold(),
        IsinRegistry::DEFAULT_ECONOMIC_THRESHOLD
    );
    let element = || {
        let mut element = order(1, &[(IdType::Fisn, "APPLE INC/SH")]);
        element.set_currency(Ccy::new("USD").unwrap(), true);
        element
    };
    let mut off = element();
    assert!(
        !registry.fill(&mut off),
        "a judgement no fill takes unasked"
    );
    assert_eq!(off.get_isincode(), None);
    registry.set_economic_match(true);
    let mut on = element();
    assert!(registry.fill(&mut on));
    assert_eq!(on.get_isincode(), Some(APPLE));
    assert!(on.get_securityids().is_derived(&IdType::Isin));
    // A stricter threshold refuses what it took.
    let mut strict = registry.try_with_economic_threshold(0.95).unwrap();
    assert!(strict.is_economic_match());
    assert!(!strict.fill(&mut element()));
    for refused in [0.0, 1.5, f64::NAN, -0.5] {
        let error = strict
            .set_economic_threshold(refused)
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!("{refused}")), "{refused}: {error}");
    }
    assert_eq!(strict.economic_threshold(), 0.95, "a refusal moves nothing");
    strict.set_economic_threshold(1.0).unwrap();
    // The settings are the registry's, never the store's: a clear keeps them.
    strict.clear();
    assert_eq!(
        (strict.economic_threshold(), strict.is_economic_match()),
        (1.0, true)
    );
    let disabled = IsinRegistry::new().with_economic_match(false);
    assert!(!disabled.is_economic_match());
}

/// The fallbacks stand beside the waterfall: the listing currency its
/// market's country implies fills an empty column only, the origin
/// currency is never derived into a column, and an element's
/// `origin_currency` reads its own currency where it holds none.
#[test]
fn the_fallbacks_fill_empty_columns_and_derive_no_origin_currency() {
    crate::install::installed();
    let mut registry = IsinRegistry::new();
    registry
        .merge(IsinEntry::new(isin(SAP)).with_miccode(mic("XETR")))
        .unwrap();
    registry
        .merge(
            IsinEntry::new(isin(DIAGEO))
                .with_miccode(mic("XLON"))
                .with_currency(ccy("USD")),
        )
        .unwrap();
    let sap = registry.get(SAP).unwrap();
    assert_eq!(sap.currency().map(Ccy::as_str), Some("EUR"));
    assert_eq!(sap.origccy(), None);
    assert_eq!(
        registry.get(DIAGEO).unwrap().currency().map(Ccy::as_str),
        Some("USD"),
        "a statement stands"
    );
    let mut element = order(1, &[(IdType::Wkn, "716460")]);
    element.set_currency(Ccy::new("EUR").unwrap(), true);
    assert!(registry.fill(&mut element));
    assert_eq!(element.get_isincode(), Some(SAP));
    assert!(element.get_origccy().is_none());
    assert_eq!(element.origin_currency().as_str(), "EUR");
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::logging_warning::count;
    use yggdryl_market::internals::isin_registry::ENTRY_CHARGE;
    use yggdryl_market::{IdType, IsinRegistry};

    use super::{NOVARTIS, SAP, entry, mic};

    /// A statement naming no market warns, once per column, of each listing
    /// fact it states where its ISIN has several listings - the fact lands
    /// on none - and says nothing where the ISIN has one, which takes it.
    #[test]
    fn a_listing_fact_of_no_market_on_several_listings_is_warned_per_column() {
        crate::install::installed();
        const SITE: &str = "yggdryl_market::isin_registry";
        const WHAT: &str = "instrument registry listing fact dropped: the statement names no market of the instrument's several listings";
        // A listing code no other test states without a market.
        let seen = || count(SITE, WHAT, "umtf");
        let before = seen();
        let mut registry = IsinRegistry::new();
        registry
            .merge(entry(SAP, Some(1), &[]).with_miccode(mic("XETR")))
            .unwrap();
        assert!(
            registry
                .merge(entry(SAP, Some(2), &[(IdType::Umtf, "U-1")]))
                .unwrap()
        );
        assert_eq!(seen(), before, "a single listing takes it");
        for market in ["XSWX", "XLON"] {
            registry
                .merge(entry(NOVARTIS, Some(1), &[]).with_miccode(mic(market)))
                .unwrap();
        }
        assert!(
            !registry
                .merge(entry(NOVARTIS, Some(2), &[(IdType::Umtf, "U-2")]))
                .unwrap()
        );
        assert_eq!(seen(), before + 1);
        assert!(
            registry
                .listings(NOVARTIS)
                .iter()
                .all(|row| row.get(&IdType::Umtf).is_none())
        );
    }

    /// A listing row is charged its worst case, 5,387 bytes on a 64-bit
    /// target since the code index joined the table - one slot per code a
    /// row holds, `12 * (80 + 96 + 16)` bytes with each value's heap - and
    /// `firstunix` the row, past the 4 KiB it was charged before, so the
    /// charge is the next power of two of KiB and the default bound holds
    /// 128 MiB rather than 64.
    #[test]
    fn the_default_bound_holds_at_most_one_hundred_twenty_eight_mebibytes() {
        crate::install::installed();
        assert_eq!(ENTRY_CHARGE, 8 * 1024);
        assert_eq!(
            IsinRegistry::DEFAULT_MAX_INSTRUMENTS * ENTRY_CHARGE,
            128 * 1024 * 1024
        );
    }
}
