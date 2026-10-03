//! `rust/src/isin_registry.rs`: one row per ISIN of every equivalent an
//! instrument is known by - learned from statements, filled into the ones
//! that leave it unsaid, updated by the latest statement column by column,
//! a RIC leading back to its ISIN - read from and written to a holder
//! through the Arrow record surface.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowType, Field as ArrowField, Schema};
use smol_str::SmolStr;
use yggdryl::graph::{Market, OrderEvent};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{
    Cfi, IOBase, IOMedia, IOMode, IdKey, IdType, Identifier, Isin, IsinEntry, IsinRegistry, Mic,
};

const HOLCIM: &str = "CH0012214059";
const APPLE: &str = "US0378331005";
const NOVARTIS: &str = "CH0012005267";

fn isin(text: &str) -> Isin {
    Isin::new(text).unwrap()
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
    let mut registry = IsinRegistry::new();
    assert!(registry.learn(&stated));
    assert!(!registry.learn(&stated), "nothing new");
    let row = registry.get(HOLCIM).expect("a row");
    assert_eq!(row.updunix(), Some(10));
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(row.get(&IdType::Bloomberg), Some("HOLN SW Equity"));
    assert_eq!(row.cficode().map(|code| code.as_str()), Some("ESVUFR"));
    assert_eq!(row.miccode().map(|code| code.as_str()), Some("XSWX"));
    assert_eq!(row.ticker(), Some("HOLN"));
    assert_eq!(
        registry.get_by_ric("HOLN.S").map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );

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
    assert!(!registry.fill(&mut named), "nothing left to fill");
    // What a fill derived is never learned back.
    assert!(!registry.learn(&named));

    // A statement of its own stands; a refined CFI code refines it.
    let mut own = order(30, &[(IdType::Isin, HOLCIM), (IdType::Common, "C-9")]);
    own.set_cficode(cfi("ESXUFR"), true);
    assert!(registry.fill(&mut own));
    assert_eq!(own.get_securityids().get(&IdType::Common), Some("C-9"));
    assert_eq!(own.get_cficode().map(|code| code.as_str()), Some("ESVUFR"));
}

#[test]
fn a_ric_leads_to_its_isin_and_learning_through_it_only_fills() {
    let mut registry = IsinRegistry::new();
    registry
        .merge(entry(
            HOLCIM,
            Some(10),
            &[(IdType::Ric, "HOLN.S"), (IdType::Common, "C-1")],
        ))
        .unwrap();
    // An element naming only the RIC takes the ISIN, derived, then the rest.
    let mut by_ric = order(20, &[(IdType::Ric, "HOLN.S")]);
    assert!(registry.fill(&mut by_ric));
    assert_eq!(by_ric.get_isincode(), Some(HOLCIM));
    assert!(by_ric.get_securityids().is_derived(&IdType::Isin));
    assert_eq!(by_ric.get_securityids().get(&IdType::Common), Some("C-1"));

    // A later statement keyed by its RIC fills a gap and replaces nothing.
    let through = order(
        30,
        &[
            (IdType::Ric, "HOLN.S"),
            (IdType::Common, "C-2"),
            (IdType::Belgian, "B-1"),
        ],
    );
    assert!(registry.learn(&through));
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Common), Some("C-1"), "never replaced");
    assert_eq!(row.get(&IdType::Belgian), Some("B-1"), "filled");
    // A RIC no row holds keys nothing.
    assert!(!registry.learn(&order(40, &[(IdType::Ric, "VOD.L")])));
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
        "a statement without an ISIN or a known RIC is learned by nothing"
    );
}

#[test]
fn the_latest_statement_leads_and_an_older_one_only_fills() {
    let mut registry = IsinRegistry::new();
    assert!(
        registry
            .merge(entry(HOLCIM, Some(10), &[(IdType::Common, "A")]))
            .unwrap()
    );
    // Older: fills what the row lacks, never replaces, never moves updunix
    // back.
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
    assert_eq!(row.get(&IdType::Common), Some("A"));
    assert_eq!(row.get(&IdType::Belgian), Some("X"));
    assert_eq!(row.updunix(), Some(10));
    // Newer, or at the same instant: replaces.
    assert!(
        registry
            .merge(entry(HOLCIM, Some(10), &[(IdType::Common, "C")]))
            .unwrap()
    );
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
    // An undated statement is older than any dated row.
    assert!(
        !registry
            .merge(entry(HOLCIM, None, &[(IdType::Common, "E")]))
            .unwrap()
    );
    // Nothing that differs moves nothing.
    assert!(
        !registry
            .merge(entry(HOLCIM, Some(99), &[(IdType::Common, "D")]))
            .unwrap()
    );
    assert_eq!(registry.get(HOLCIM).unwrap().updunix(), Some(20));

    // An undated row is the oldest.
    let mut undated = IsinRegistry::new();
    undated
        .merge(entry(APPLE, None, &[(IdType::Common, "A")]))
        .unwrap();
    assert!(
        undated
            .merge(entry(APPLE, None, &[(IdType::Common, "B")]))
            .unwrap()
    );
    assert_eq!(undated.get(APPLE).unwrap().get(&IdType::Common), Some("B"));
}

/// A column takes the higher-ranked value whatever the time: a code its
/// check digit closes replaces a typo from an older statement, a typo from
/// a newer one never replaces it, and only between two of one rank does the
/// time decide.
#[test]
fn a_column_never_downgrades_and_upgrades_whatever_the_time() {
    const REAL: &str = "037833100";
    const TYPO: &str = "037833101";
    const OTHER: &str = "594918104";
    let mut registry = IsinRegistry::new();
    assert!(
        registry
            .merge(entry(HOLCIM, Some(20), &[(IdType::Cusip, TYPO)]))
            .unwrap()
    );
    // An older real code upgrades the typo.
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
    // A newer typo never downgrades it.
    assert!(
        !registry
            .merge(entry(HOLCIM, Some(30), &[(IdType::Cusip, TYPO)]))
            .unwrap()
    );
    assert_eq!(
        registry.get(HOLCIM).unwrap().get(&IdType::Cusip),
        Some(REAL)
    );
    // Two real codes: the newer leads, the older only fills.
    assert!(
        !registry
            .merge(entry(HOLCIM, Some(15), &[(IdType::Cusip, OTHER)]))
            .unwrap()
    );
    assert!(
        registry
            .merge(entry(HOLCIM, Some(40), &[(IdType::Cusip, OTHER)]))
            .unwrap()
    );
    assert_eq!(
        registry.get(HOLCIM).unwrap().get(&IdType::Cusip),
        Some(OTHER)
    );

    // The key is a real number: a masked or mistyped one names no row.
    for unreal in ["XX0000000001", "CH0012214058", "ZZ0000000008"] {
        let refused = IsinRegistry::new()
            .merge(IsinEntry::new(isin(unreal)))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("$.isin"), "{unreal}: {refused}");
        assert!(refused.contains(unreal), "{unreal}: {refused}");
    }
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
    // A real ISIN stated beside a ticker another row lists stands.
    let mut stated = order(20, &[(IdType::Isin, APPLE)]);
    stated.set_ticker(Some(SmolStr::new("HOLN")), true);
    assert!(!registry.fill(&mut stated));
    assert_eq!(stated.get_isincode(), Some(APPLE));

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
    // The index follows a row's ticker: a newer listing fact moves it, and
    // a removed row leaves it.
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
fn a_cfi_code_refines_whatever_the_time_and_a_conflict_goes_to_the_latest() {
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
        !registry.merge(classified(5, "ESNUFR")).unwrap(),
        "an older conflict is dropped"
    );
    assert!(registry.merge(classified(20, "ESNUFR")).unwrap());
    assert_eq!(held(&registry), "ESNUFR", "a newer one replaces whole");
    // A coarse code is stored as none.
    assert!(
        IsinEntry::new(isin(HOLCIM))
            .with_cficode(cfi("ESXXXX"))
            .cficode()
            .is_none()
    );
}

#[test]
fn a_newer_listing_fact_on_another_market_switches_the_listing_whole() {
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
            .with_ticker(Some(SmolStr::new("HOLN"))),
        )
        .unwrap();
    // Older on another market: its listing facts are dropped.
    assert!(
        !registry
            .merge(listing(5, "XLON", &[(IdType::Ric, "HOLN.L")]))
            .unwrap()
    );
    // Newer with a listing fact: the listing switches, every listing column
    // it does not restate cleared, the instrument's columns kept.
    assert!(
        registry
            .merge(listing(20, "XLON", &[]).with_ticker(Some(SmolStr::new("HOLNL"))))
            .unwrap()
    );
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.miccode().map(|code| code.as_str()), Some("XLON"));
    assert_eq!(row.ticker(), Some("HOLNL"));
    assert_eq!(row.get(&IdType::Ric), None);
    assert_eq!(row.get(&IdType::Common), Some("C"));
    assert!(registry.get_by_ric("HOLN.S").is_none(), "the index follows");
    // No market: listing facts under the row's.
    assert!(
        registry
            .merge(entry(HOLCIM, Some(30), &[(IdType::Ric, "HOLN.L")]))
            .unwrap()
    );
    assert_eq!(
        registry.get_by_ric("HOLN.L").map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );

    // A fill never carries one market's listing onto another's message.
    let mut elsewhere = order(40, &[(IdType::Isin, HOLCIM)]);
    elsewhere.set_miccode(mic("XSWX"), true);
    assert!(registry.fill(&mut elsewhere));
    assert_eq!(elsewhere.get_securityids().get(&IdType::Ric), None);
    assert_eq!(elsewhere.get_ticker(), None);
    assert_eq!(
        elsewhere.get_securityids().get(&IdType::Common),
        Some("C"),
        "the instrument's own columns fill everywhere"
    );
}

#[test]
fn a_ric_names_one_listing_and_moves_only_to_a_later_statement() {
    let mut registry = IsinRegistry::new();
    registry
        .merge(entry(HOLCIM, Some(10), &[(IdType::Ric, "R.X")]))
        .unwrap();
    // An older statement of the RIC under another ISIN does not take it.
    registry
        .merge(entry(
            APPLE,
            Some(5),
            &[(IdType::Ric, "R.X"), (IdType::Common, "A")],
        ))
        .unwrap();
    assert_eq!(registry.get(APPLE).unwrap().get(&IdType::Ric), None);
    assert_eq!(
        registry.get_by_ric("R.X").map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );
    // A later one does, and the row it left holds none.
    registry
        .merge(entry(APPLE, Some(15), &[(IdType::Ric, "R.X")]))
        .unwrap();
    assert_eq!(
        registry.get_by_ric("R.X").map(|row| row.isin().as_str()),
        Some(APPLE)
    );
    assert_eq!(registry.get(HOLCIM).unwrap().get(&IdType::Ric), None);
    // The index is the exact inverse: a moved RIC leaves its old key.
    registry
        .merge(entry(APPLE, Some(20), &[(IdType::Ric, "R.Y")]))
        .unwrap();
    assert!(registry.get_by_ric("R.X").is_none());
    assert!(registry.get_by_ric("R.Y").is_some());
    assert!(registry.remove(APPLE).is_some());
    assert!(registry.get_by_ric("R.Y").is_none());
}

/// One statement both switching a row's listing to another market and
/// stating a RIC a newer row holds leaves the switched row with no RIC: the
/// old market's RIC never rides onto the new one, so a fill on the new
/// market derives none.
#[test]
fn a_listing_switch_keeps_no_ric_of_the_market_it_left() {
    let mut registry = IsinRegistry::new();
    let listing = |text: &str, unix: i64, market: &str, ric: &str| {
        entry(text, Some(unix), &[(IdType::Ric, ric)]).with_miccode(mic(market))
    };
    registry
        .merge(listing(HOLCIM, 10, "XSWX", "HOLN.S"))
        .unwrap();
    registry
        .merge(listing(APPLE, 50, "XLON", "HOLN.L"))
        .unwrap();
    // Newer than Holcim's row, older than the row holding `HOLN.L`.
    assert!(
        registry
            .merge(listing(HOLCIM, 20, "XLON", "HOLN.L"))
            .unwrap()
    );
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.miccode().map(|code| code.as_str()), Some("XLON"));
    assert_eq!(row.get(&IdType::Ric), None, "SIX's RIC left with SIX");
    assert!(registry.get_by_ric("HOLN.S").is_none(), "the index follows");
    assert_eq!(
        registry.get_by_ric("HOLN.L").map(|row| row.isin().as_str()),
        Some(APPLE)
    );
    let mut london = order(60, &[(IdType::Isin, HOLCIM)]);
    london.set_miccode(mic("XLON"), true);
    registry.fill(&mut london);
    assert_eq!(london.get_securityids().get(&IdType::Ric), None);

    // On the market it names, the row's own RIC stays when a statement's
    // RIC is not taken.
    registry
        .merge(listing(HOLCIM, 30, "XLON", "HOLN.X"))
        .unwrap();
    assert!(
        !registry
            .merge(listing(HOLCIM, 40, "XLON", "HOLN.L"))
            .unwrap(),
        "a RIC a newer row holds moves nothing"
    );
    assert_eq!(
        registry.get(HOLCIM).unwrap().get(&IdType::Ric),
        Some("HOLN.X")
    );
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
    let unknown = Isin::new(format!(
        "ZZ000000000{}",
        Isin::closing_digit("ZZ000000000").unwrap()
    ))
    .unwrap();
    assert!(IsinRegistry::new().merge(IsinEntry::new(unknown)).is_err());
}

#[test]
fn a_clone_is_a_snapshot_a_write_does_not_reach() {
    let mut registry = IsinRegistry::new();
    registry
        .merge(entry(HOLCIM, Some(1), &[(IdType::Common, "A")]))
        .unwrap();
    let snapshot = registry.clone();
    registry
        .merge(entry(APPLE, Some(1), &[(IdType::Common, "B")]))
        .unwrap();
    registry
        .merge(entry(HOLCIM, Some(2), &[(IdType::Common, "C")]))
        .unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(
        snapshot.get(HOLCIM).unwrap().get(&IdType::Common),
        Some("A")
    );
    assert_eq!(registry.len(), 2);
    registry.clear();
    assert!(registry.is_empty());
    assert_eq!(snapshot.len(), 1);
}

#[test]
fn an_entry_reads_back_from_its_scalar() {
    let entry = entry(
        HOLCIM,
        Some(7),
        &[(IdType::Ric, "HOLN.S"), (IdType::Valor, "1221405")],
    )
    .with_cficode(cfi("ESVUFR"))
    .with_miccode(mic("XSWX"))
    .with_ticker(Some(SmolStr::new(" HOLN ")));
    assert_eq!(entry.ticker(), Some("HOLN"));
    assert_eq!(IsinEntry::from_scalar(&entry.into_scalar()).unwrap(), entry);
    let columns: Vec<String> = IsinEntry::dtype()
        .as_fields()
        .unwrap()
        .iter()
        .map(|field| field.name().to_string())
        .collect();
    assert_eq!(columns.len(), 37);
    assert_eq!(
        &columns[..7],
        [
            "isin", "updunix", "cficode", "miccode", "ticker", "cusip", "sedol"
        ]
    );
    assert!(!IsinEntry::field().is_nullable());
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
            .with_miccode(mic("XSWX")),
        )
        .unwrap();
    registry
        .merge(entry(APPLE, None, &[(IdType::Cusip, "037833100")]))
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
    let back = IsinRegistry::from_handle(&handle).unwrap();
    assert!(back.iter().eq(registry.iter()));
    assert_eq!(
        back.get_by_ric("HOLN.S").map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );
    // Folding the same rows again moves nothing.
    let mut again = back.clone();
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
    let back = IsinRegistry::from_handle(&handle).unwrap();
    assert!(back.iter().eq(registry.iter()));
    assert_eq!(
        back.get_by_ric("HOLN.S").map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );
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
    let back = IsinRegistry::from_handle(&handle).unwrap();
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

/// A folder of parts reads as one stream, its rows of one ISIN folded by
/// `updunix` whichever part holds them: a part written by an overwrite and
/// grown by an append, and a second part of older rows, which only fill.
#[test]
fn a_folder_of_parts_loads_folded_by_updunix() {
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
    assert_eq!(holcim.updunix(), Some(30));
    assert_eq!(holcim.miccode().map(|code| code.as_str()), Some("XLON"));
    assert_eq!(holcim.get(&IdType::Ric), Some("HOLN.L"), "the latest leads");
    assert_eq!(
        holcim.get(&IdType::Common),
        Some("C-5"),
        "an older row fills"
    );
    assert_eq!(holcim.get(&IdType::Valor), Some("1221405"));
    assert_eq!(
        loaded.get_by_ric("HOLN.L").map(|row| row.isin().as_str()),
        Some(HOLCIM)
    );
    assert!(loaded.get_by_ric("HOLN.S").is_none());
    assert!(loaded.get(APPLE).is_some());
    // The folder's own record stream is the same door.
    let from_folder = IsinRegistry::from_handle(&folder).unwrap();
    assert!(from_folder.iter().eq(loaded.iter()));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_missing_store_reads_as_the_empty_registry() {
    let missing = Buffer::new().with_media_type(yggdryl::MimeType::ARROW_STREAM.into());
    let registry = IsinRegistry::from_handle(&missing).unwrap();
    assert!(registry.is_empty());
}

#[test]
fn a_flat_golden_file_loads_by_the_columns_its_names_spell() {
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("ISIN", ArrowType::Utf8, false),
        ArrowField::new("RIC", ArrowType::Utf8, true),
        ArrowField::new("CFI", ArrowType::Utf8, true),
        ArrowField::new("BloombergSymbol", ArrowType::Utf8, true),
        ArrowField::new("MIC", ArrowType::Utf8, true),
        ArrowField::new("rank", ArrowType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from(vec![HOLCIM, APPLE])),
            Arc::new(StringArray::from(vec![Some("HOLN.S"), None])),
            Arc::new(StringArray::from(vec![Some("ESVUFR"), None])),
            Arc::new(StringArray::from(vec![Some("HOLN SW Equity"), None])),
            Arc::new(StringArray::from(vec![Some("XSWX"), None])),
            Arc::new(Int64Array::from(vec![1, 2])),
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
    assert!(registry.get(APPLE).unwrap().iter().next().is_none());

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
    ]))
    .unwrap();
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Ric), None);
    assert_eq!(row.ticker(), None);
    assert_eq!(row.get(&IdType::Valor), Some("1221405"));
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
