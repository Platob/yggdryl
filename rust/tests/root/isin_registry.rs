//! `rust/src/isin_registry.rs`: one row per ISIN of every fact an
//! instrument is known by - learned from statements, filled into the ones
//! that leave it unsaid, a valid value filling and replacing whatever the
//! time, a ticker leading back to its ISIN on its market - read from and
//! written to a holder through the Arrow record surface.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowType, Field as ArrowField, Schema};
use smol_str::SmolStr;
use yggdryl::graph::{Market, OrderEvent};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{
    Ccy, Cfi, Country, Forex, IOBase, IOMedia, IOMode, IdKey, IdType, Identifier, Isin, IsinEntry,
    IsinRegistry, Mic,
};

const HOLCIM: &str = "CH0012214059";
const APPLE: &str = "US0378331005";
const NOVARTIS: &str = "CH0012005267";

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

#[test]
fn a_statement_is_learned_by_its_isin_and_filled_into_one_naming_it() {
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
    // What a fill derived is never learned back.
    assert!(!registry.learn(&named));

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

/// A RIC is a listing code like a Bloomberg symbol: filled on the same
/// market, never a key a lookup or a learn reads.
#[test]
fn a_ric_is_an_ordinary_listing_code_and_keys_nothing() {
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
    // An element naming only the RIC resolves to nothing.
    let mut by_ric = order(20, &[(IdType::Ric, "HOLN.S")]);
    assert!(!registry.fill(&mut by_ric));
    assert_eq!(by_ric.get_isincode(), None);
    // A statement naming a RIC and no ISIN is learned by nothing.
    assert!(!registry.learn(&order(
        30,
        &[(IdType::Ric, "HOLN.S"), (IdType::Belgian, "B-1")]
    )));
    assert_eq!(registry.get(HOLCIM).unwrap().get(&IdType::Belgian), None);
    // Two rows may hold one RIC, and a fill hands each its own.
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

#[test]
fn a_bloomberg_symbol_is_never_a_key() {
    let mut registry = IsinRegistry::new();
    registry
        .merge(entry(
            HOLCIM,
            Some(10),
            &[(IdType::Bloomberg, "HOLN SW Equity")],
        ))
        .unwrap();
    let mut symbol = order(20, &[(IdType::Bloomberg, "HOLN SW Equity")]);
    assert!(!registry.fill(&mut symbol));
    assert_eq!(symbol.get_isincode(), None);
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
}

/// No clock gates a merge: a stated valid value fills a column the row
/// lacks and replaces one it holds that differs, an older or undated
/// statement included; the same value moves nothing; `updunix` is the
/// later of the two, a stamp.
#[test]
fn a_valid_statement_fills_and_replaces_whatever_the_time() {
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
    assert!(moved.remove(APPLE).is_some());
    assert!(moved.remove(APPLE).is_none());
    assert!(moved.is_dirty());
}

/// Only a real value moves a column: a typo under a checked code is
/// dropped from the statement, whatever its date, and a real value
/// replaces a real one; the key is a real ISIN.
#[test]
fn an_invalid_value_moves_nothing_and_a_real_one_replaces() {
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
    assert!(registry.remove(HOLCIM).is_some());
    assert!(registry.get_by_ticker("HOLN", None).is_none());
    registry.clear();
    assert!(registry.get_by_ticker("NOVN", None).is_none());
}

#[test]
fn a_cfi_code_refines_and_a_conflict_replaces_whatever_the_time() {
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

/// A ticker or a listing code stated on another market switches the
/// listing whole - market, ticker, currency, listing codes - whatever the
/// time, clearing what it does not restate; a currency alone on another
/// market moves nothing; the instrument's own columns stay.
#[test]
fn a_listing_fact_on_another_market_switches_the_listing_whole() {
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
    // A currency alone on another market names no listing.
    assert!(
        !registry
            .merge(listing(20, "XLON", &[]).with_currency(ccy("GBP")))
            .unwrap()
    );
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.miccode().map(|code| code.as_str()), Some("XSWX"));
    assert_eq!(row.currency().map(|code| code.as_str()), Some("CHF"));
    // A listing fact there - older, even - switches the listing whole.
    assert!(
        registry
            .merge(listing(5, "XLON", &[(IdType::Ric, "HOLN.L")]))
            .unwrap()
    );
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.miccode().map(|code| code.as_str()), Some("XLON"));
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.L"));
    assert_eq!(row.ticker(), None, "cleared: the statement restated none");
    assert_eq!(row.currency(), None);
    assert_eq!(row.get(&IdType::Common), Some("C"), "the instrument's");
    assert_eq!(row.updunix(), Some(10));
    assert!(
        registry.get_by_ticker("HOLN", None).is_none(),
        "the index follows"
    );
    // Back with a ticker and a currency: the listing switches again.
    assert!(
        registry
            .merge(
                listing(30, "XSWX", &[])
                    .with_ticker(Some(SmolStr::new("HOLN")))
                    .with_currency(ccy("CHF"))
            )
            .unwrap()
    );
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.miccode().map(|code| code.as_str()), Some("XSWX"));
    assert_eq!(row.get(&IdType::Ric), None);
    assert_eq!(row.ticker(), Some("HOLN"));
    assert_eq!(row.currency().map(|code| code.as_str()), Some("CHF"));
    // No market: listing facts fold under the row's.
    assert!(
        registry
            .merge(entry(HOLCIM, Some(40), &[(IdType::Ric, "HOLN.S")]))
            .unwrap()
    );
    assert_eq!(
        registry.get(HOLCIM).unwrap().get(&IdType::Ric),
        Some("HOLN.S")
    );

    // A fill never carries one market's listing onto another's message.
    let mut elsewhere = order(50, &[(IdType::Isin, HOLCIM)]);
    elsewhere.set_miccode(mic("XLON"), true);
    assert!(registry.fill(&mut elsewhere));
    assert_eq!(elsewhere.get_securityids().get(&IdType::Ric), None);
    assert_eq!(elsewhere.get_ticker(), None);
    assert!(elsewhere.get_currency().is_none());
    assert_eq!(
        elsewhere.get_securityids().get(&IdType::Common),
        Some("C"),
        "the instrument's own columns fill everywhere"
    );
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
    assert_eq!(row.currency(), None, "XXX states no currency");
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
    assert!(
        !registry.learn(&later),
        "a derived pair is never learned back"
    );
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
    // nothing else is left to.
    let mut other_ticker = order(20, &[(IdType::Isin, HOLCIM)]);
    other_ticker.set_miccode(mic("XSWX"), true);
    other_ticker.set_ticker(Some(SmolStr::new("HOLNX")), true);
    assert!(!registry.fill(&mut other_ticker));
    assert!(other_ticker.get_currency().is_none());
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
    assert!(!holding.learn(&order(
        99,
        &[(IdType::Isin, HOLCIM), (IdType::RedPair, "X-1")]
    )));
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
    let entry = entry(
        HOLCIM,
        Some(7),
        &[(IdType::Ric, "HOLN.S"), (IdType::Valor, "1221405")],
    )
    .with_cficode(cfi("ESVUFR"))
    .with_countrycode(Some(Country::new("CH").unwrap()))
    .with_forexcode(Some(Forex::new("EUR/CHF").unwrap()))
    .with_underlyingisin(Some(isin(APPLE)))
    .with_miccode(mic("XSWX"))
    .with_ticker(Some(SmolStr::new(" HOLN ")))
    .with_currency(ccy("CHF"));
    assert_eq!(entry.ticker(), Some("HOLN"));
    assert_eq!(entry.underlyingisin().map(Isin::as_str), Some(APPLE));
    assert_eq!(IsinEntry::from_scalar(&entry.into_scalar()).unwrap(), entry);
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
    assert_eq!(columns.len(), 41);
    assert_eq!(
        &columns[..10],
        [
            "isin",
            "updunix",
            "cficode",
            "countrycode",
            "forexcode",
            "underlyingisin",
            "miccode",
            "ticker",
            "currency",
            "cusip"
        ]
    );
    assert!(!IsinEntry::field().is_nullable());
    let types: Vec<String> = IsinEntry::dtype()
        .as_fields()
        .unwrap()
        .iter()
        .take(9)
        .map(|field| field.dtype().to_string())
        .collect();
    assert_eq!(
        types,
        [
            "isin",
            "datetime64(ns,\"UTC\")",
            "cfi",
            "country",
            "forex",
            "isin",
            "mic",
            "utf8",
            "ccy"
        ]
    );
}

/// The underlying is an instrument fact: a real ISIN other than the row's
/// own fills and replaces on any market and no listing switch clears it; the
/// row's own ISIN or a typo states nothing; `learn` never states one, since
/// what a FIX message names as its underlying is the lifecycle's reading.
#[test]
fn the_underlying_is_an_instrument_fact_merged_by_the_update_rule() {
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
    // A listing switch on another market keeps it: an instrument fact.
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
    assert_eq!(row.underlyingisin().map(Isin::as_str), Some(NOVARTIS));
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

#[test]
fn a_registry_round_trips_an_ipc_file_through_the_record_surface() {
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
                .with_forexcode(Some(Forex::new("USD/CHF").unwrap())),
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
/// and grown by an append, then a second part of older rows, which still
/// fill and replace.
#[test]
fn a_folder_of_parts_loads_in_part_order() {
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
    let holcim = loaded.get(HOLCIM).unwrap();
    assert_eq!(holcim.updunix(), Some(30), "the stamp is the latest");
    assert_eq!(
        holcim.miccode().map(|code| code.as_str()),
        Some("XLON"),
        "the first part's appended listing switched the market"
    );
    assert_eq!(
        holcim.get(&IdType::Ric),
        Some("HOLN.Z"),
        "the second part's RIC, folded under the row's market, replaced"
    );
    assert_eq!(holcim.get(&IdType::Common), Some("C-5"), "filled");
    assert_eq!(holcim.get(&IdType::Valor), Some("1221405"));
    assert!(loaded.get(APPLE).is_some());
    // The folder's own record stream is the same door.
    let mut from_folder = IsinRegistry::new();
    from_folder.extend_from_handle(&folder).unwrap();
    assert!(from_folder.iter().eq(loaded.iter()));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_missing_store_reads_as_the_empty_registry() {
    let missing = Buffer::new().with_media_type(yggdryl::MimeType::ARROW_STREAM.into());
    let mut registry = IsinRegistry::new();
    assert_eq!(registry.extend_from_handle(&missing).unwrap(), 0);
    assert!(registry.is_empty());
    assert!(!registry.is_dirty());
}

#[test]
fn a_flat_golden_file_loads_by_the_columns_its_names_spell() {
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
    assert!(apple.iter().next().is_none());
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
    assert_eq!(row.currency(), None);
    assert_eq!(row.get(&IdType::Valor), Some("1221405"));
}

/// An LEI and a DTI equivalent are typed by their own codes, so a store
/// declares them and a round trip keeps them; a store whose column is
/// `utf8` still loads, its cells cast into the code, a cell that is not the
/// code's canonical spelling landing null.
#[test]
fn an_lei_and_a_dti_column_is_its_own_code_and_a_utf8_one_still_loads() {
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

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::IsinRegistry;
    use yggdryl::internals::isin_registry::ENTRY_CHARGE;

    #[test]
    fn the_default_bound_holds_at_most_forty_eight_mebibytes() {
        assert_eq!(ENTRY_CHARGE, 3 * 1024);
        assert_eq!(
            IsinRegistry::DEFAULT_MAX_INSTRUMENTS * ENTRY_CHARGE,
            48 * 1024 * 1024
        );
    }
}
