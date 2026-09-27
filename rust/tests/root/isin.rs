//! `rust/src/isin.rs`: the ISO 6166 securities identification number, and
//! how two statements of one instrument's number fold.
//!
//! ISO 6166:2021 gives `ZZ` to derivatives numbered before a country or
//! the DSB's `EZ` numbers them, so a `ZZ` number is the placeholder a real
//! one replaces; every other prefix is a number of its own.

use yggdryl::{CodeValue, Isin};

/// The checksum-valid number eleven leading characters close to.
fn closed(body: &str) -> Isin {
    let digit = Isin::closing_digit(body).expect("two letters and nine alphanumerics");
    Isin::new(format!("{body}{digit}")).unwrap()
}

#[test]
fn a_zz_number_yields_to_any_other_prefix_and_every_other_number_stands() {
    let apple = Isin::new("US0378331005").unwrap();
    let microsoft = Isin::new("US5949181045").unwrap();
    let provisional = closed("ZZ000A0B1C2");
    let other_provisional = closed("ZZ000000001");
    let dsb = closed("EZ1234567AB");
    assert_eq!(provisional.prefix(), "ZZ");
    assert_eq!(dsb.prefix(), "EZ");

    // The placeholder takes the real number, whichever real number it is.
    assert_eq!(provisional.clone().merge_with(&apple), apple);
    assert_eq!(provisional.clone().merge_with(&dsb), dsb);
    // A real number never takes the placeholder.
    assert_eq!(apple.clone().merge_with(&provisional), apple);
    assert_eq!(dsb.clone().merge_with(&provisional), dsb);
    // Two placeholders, or two real numbers, are two statements and this one
    // stands: a DSB number and a country's are both real.
    assert_eq!(
        provisional.clone().merge_with(&other_provisional),
        provisional
    );
    assert_eq!(apple.clone().merge_with(&microsoft), apple);
    assert_eq!(dsb.clone().merge_with(&apple), dsb);
    assert_eq!(apple.clone().merge_with(&dsb), apple);
}

#[test]
fn intake_is_unchanged_by_the_merge_rule() {
    // A number that does not close stays refused whatever its prefix.
    assert!(Isin::new("XX0000000001").is_err());
    let provisional = closed("ZZ000000001");
    assert!(Isin::is_canonical(provisional.as_str()));
    assert_eq!(
        Isin::new(provisional.as_str().to_ascii_lowercase()).unwrap(),
        provisional
    );
}
