//! `rust/market/src/characteristics.rs`: the typed body facts a `class:body`
//! cross code is written from - a settle as a date or a tenor, an expiry as
//! a day, a month or a week, an exercise style - each spelled as the key
//! spells it and read back from that text, and the struct of them crossing
//! its scalar.

use yggdryl::{Decimal, Scalar};
use yggdryl_market::{Characteristics, Exercise, Expiry, Settle};

/// A settle is a date or a FIX `SettlType(63)` tenor, the two never
/// colliding, spelled upper case as the key spells it.
#[test]
fn a_settle_is_a_date_or_a_tenor_spelled_as_the_key_spells_it() {
    crate::install::installed();
    for (text, spelled) in [
        ("2027-01-15", "2027-01-15"),
        ("20270115", "2027-01-15"),
        ("0", "0"),
        ("m3", "M3"),
        ("W1", "W1"),
        ("C", "C"),
    ] {
        let settle: Settle = text.parse().unwrap();
        assert_eq!(settle.to_string(), spelled, "{text}");
        assert_eq!(spelled.parse::<Settle>().unwrap(), settle, "{text}");
    }
    assert!(matches!(
        "2027-01-15".parse::<Settle>().unwrap(),
        Settle::Date(_)
    ));
    assert!(matches!("M3".parse::<Settle>().unwrap(), Settle::Tenor(_)));
    for refused in ["", "TOOLONG", "2027-13-01", "M-3", "2027-01-15T10:00"] {
        let error = Settle::from_text(refused).unwrap_err().to_string();
        assert!(error.contains("$.settle"), "{refused}: {error}");
    }
}

/// An expiry is a day, a contract month or a week of one; a day's month is
/// the month production a future is keyed by.
#[test]
fn an_expiry_is_a_day_a_month_or_a_week() {
    crate::install::installed();
    for (text, spelled) in [
        ("2026-12-18", "2026-12-18"),
        ("20261218", "2026-12-18"),
        ("2026-12", "2026-12"),
        ("202612", "2026-12"),
        ("2026-12w3", "2026-12w3"),
        ("202612w3", "2026-12w3"),
    ] {
        let expiry: Expiry = text.parse().unwrap();
        assert_eq!(expiry.to_string(), spelled, "{text}");
        assert_eq!(spelled.parse::<Expiry>().unwrap(), expiry, "{text}");
    }
    let day: Expiry = "2026-12-18".parse().unwrap();
    assert_eq!(day.month().to_string(), "2026-12");
    let week: Expiry = "2026-12w3".parse().unwrap();
    assert_eq!(week.month(), week, "a month is its own month");
    assert_eq!(
        week,
        Expiry::Month {
            year: 2026,
            month: 12,
            week: Some(3)
        }
    );
    for refused in [
        "",
        "2026-13",
        "2026-12w6",
        "2026-12w0",
        "2026-02-30",
        "26-12",
        "2026/12",
    ] {
        let error = Expiry::from_text(refused).unwrap_err().to_string();
        assert!(error.contains("$.expiry"), "{refused}: {error}");
    }
}

/// The exercise style reads FIX's code and its name, in any case.
#[test]
fn an_exercise_style_reads_its_fix_code_and_its_name() {
    crate::install::installed();
    for (style, code, name) in [
        (Exercise::European, "0", "European"),
        (Exercise::American, "1", "American"),
        (Exercise::Bermuda, "2", "Bermuda"),
    ] {
        assert_eq!(Exercise::from_fix(code), Some(style));
        assert_eq!(style.fix_code(), code);
        assert_eq!(style.as_str(), name);
        assert_eq!(
            name.to_ascii_lowercase().parse::<Exercise>().unwrap(),
            style
        );
        assert_eq!(code.parse::<Exercise>().unwrap(), style);
    }
    assert_eq!(Exercise::from_fix("3"), None);
    assert!("Asian".parse::<Exercise>().is_err());
    assert_eq!(Exercise::ALL.len(), 3);
}

/// The characteristics cross their scalar - a stated field as its text or
/// its decimal, an unstated one null, a null whole the default - and read
/// back equal; a text that spells no settle, expiry or style is refused.
#[test]
fn the_characteristics_cross_their_scalar_and_read_back() {
    crate::install::installed();
    let option = Characteristics::default()
        .with_settle(Some("M3".parse().unwrap()))
        .with_settle2(Some("2027-01-15".parse().unwrap()))
        .with_expiry(Some("2026-12-18".parse().unwrap()))
        .with_strikepx(Some(Decimal::parse("200.5").unwrap()))
        .with_multiplier(Some(Decimal::parse("100").unwrap()))
        .with_exercise(Some(Exercise::American));
    assert!(!option.is_default());
    assert!(Characteristics::default().is_default());
    let scalar = option.into_scalar();
    let cells = scalar.as_struct().unwrap();
    assert_eq!(cells.get("settle").and_then(Scalar::as_str), Some("M3"));
    assert_eq!(
        cells.get("settle2").and_then(Scalar::as_str),
        Some("2027-01-15")
    );
    assert_eq!(
        cells.get("expiry").and_then(Scalar::as_str),
        Some("2026-12-18")
    );
    assert_eq!(
        cells.get("strikepx"),
        Some(&Scalar::from(Decimal::parse("200.5").unwrap()))
    );
    assert_eq!(
        cells.get("exercise").and_then(Scalar::as_str),
        Some("American")
    );
    assert_eq!(Characteristics::from_scalar(&scalar).unwrap(), option);
    assert_eq!(
        Characteristics::from_scalar(&Scalar::Null).unwrap(),
        Characteristics::default()
    );
    // A name the struct lacks is a null.
    let partial =
        Scalar::from_struct([("strikepx", Scalar::from(Decimal::parse("200").unwrap()))]).unwrap();
    assert_eq!(
        Characteristics::from_scalar(&partial).unwrap(),
        Characteristics::default().with_strikepx(Some(Decimal::parse("200").unwrap()))
    );
    let unread = Scalar::from_struct([("expiry", Scalar::from("someday"))]).unwrap();
    assert!(Characteristics::from_scalar(&unread).is_err());
    // The datatype: six nullable columns.
    let dtype = Characteristics::dtype();
    let names: Vec<&str> = dtype
        .as_fields()
        .unwrap()
        .iter()
        .map(|field| field.name())
        .collect();
    assert_eq!(
        names,
        [
            "settle",
            "settle2",
            "expiry",
            "strikepx",
            "multiplier",
            "exercise"
        ]
    );
    assert!(Characteristics::field().is_nullable());
}
