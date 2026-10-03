//! `rust/src/isin.rs`: the ISO 6166 securities identification number, and
//! how two statements of one instrument's number fold.
//!
//! The shape is what `new` admits - twelve characters, two letters, nine
//! alphanumerics and a digit - and whether the number closes on its check
//! digit and whether its prefix is one an agency numbers under are two
//! readings a value answers rather than two refusals: a masked or mistyped
//! number is a value of a lower rank, which every merge replaces by a real
//! one whatever the order, and only two numbers of one rank fold by the
//! order they were stated in. ISO 6166:2021 gives `ZZ` to derivatives
//! numbered before a country or the DSB's `EZ` numbers them, so a `ZZ`
//! number closes but is listed under no agency: rank one, below a real
//! number and above a number that does not close.

use yggdryl::{CodeValue, Isin};

/// The checksum-valid number eleven leading characters close to.
fn closed(body: &str) -> Isin {
    let digit = Isin::closing_digit(body).expect("two letters and nine alphanumerics");
    Isin::new(format!("{body}{digit}")).unwrap()
}

/// A number eleven leading characters do not close to: the next digit.
fn open(body: &str) -> Isin {
    let digit = Isin::closing_digit(body).expect("two letters and nine alphanumerics");
    Isin::new(format!("{body}{}", (digit + 1) % 10)).unwrap()
}

#[test]
fn the_shape_is_admitted_and_the_closing_and_the_listing_are_readings() {
    // A number that does not close is a value: a masked line's, a typo's.
    let masked = Isin::new("XX0000000001").unwrap();
    assert_eq!(masked.as_str(), "XX0000000001");
    assert!(!Isin::is_closed(masked.as_str()));
    assert!(!Isin::is_listed_prefix(masked.as_str()));
    assert_eq!(masked.rank(), 0);
    assert!(!masked.is_real());
    let typo = Isin::new("us0378331006").unwrap();
    assert_eq!(typo.as_str(), "US0378331006", "folded as every number is");
    assert!(!Isin::is_closed(typo.as_str()));
    assert!(Isin::is_listed_prefix(typo.as_str()));
    assert_eq!(typo.rank(), 1);
    // The shape is the refusal.
    for (text, reason) in [
        ("US037833100", "expected twelve characters"),
        ("U10378331005", "expected a two-letter prefix"),
        ("US03783310*5", "expected nine alphanumerics"),
        ("US037833100A", "expected a closing check digit"),
    ] {
        let refused = Isin::new(text).unwrap_err().to_string();
        assert!(refused.contains(reason), "{text}: {refused}");
        assert_eq!(Isin::rank_of(text), 0, "{text}");
    }

    // A real number closes under a listed prefix: a country's, or an
    // agency's - the DSB's `EZ`, the international `XS`.
    let apple = Isin::new("US0378331005").unwrap();
    assert!(Isin::is_closed(apple.as_str()));
    assert!(Isin::is_listed_prefix(apple.as_str()));
    assert_eq!(apple.rank(), 2);
    assert!(apple.is_real());
    assert_eq!(<Isin as CodeValue>::MAX_RANK, 2);
    for agency in ["EZ1234567AB", "XS020347015", "EU000000000"] {
        let number = closed(agency);
        assert!(Isin::is_listed_prefix(number.as_str()), "{agency}");
        assert_eq!(number.rank(), 2, "{agency}");
    }
    // A `ZZ` number closes but no agency lists it: one of the two.
    let provisional = closed("ZZ000A0B1C2");
    assert!(Isin::is_closed(provisional.as_str()));
    assert!(!Isin::is_listed_prefix(provisional.as_str()));
    assert_eq!(provisional.rank(), 1);
    assert_eq!(Isin::rank_of("ZZ0000000008"), 1);
    // The readings answer the text without building a value, and lower
    // case closes nothing: it is not how a column spells a number.
    assert_eq!(Isin::rank_of("US0378331005"), 2);
    assert_eq!(Isin::rank_of("us0378331005"), 0);
    assert!(!Isin::is_closed("us0378331005"));
}

#[test]
fn the_placeholder_is_the_lowest_number_there_is() {
    assert_eq!(Isin::NONE, "XX0000000000");
    let none = Isin::none();
    assert_eq!(none.as_str(), Isin::NONE);
    assert!(none.is_none());
    assert!(!Isin::new("XX0000000001").unwrap().is_none());
    assert_eq!(none.rank(), 0);
    assert!(!none.is_real());
    assert!(!Isin::is_closed(Isin::NONE));
    assert!(!Isin::is_listed_prefix(Isin::NONE));
    // Any stated number replaces it, whichever leads.
    let masked = Isin::new("XX0000000001").unwrap();
    assert_eq!(
        none.clone().merge_with(&masked),
        none,
        "two of rank zero: this one"
    );
    let typo = open("US037833100");
    assert_eq!(none.clone().merge_with(&typo), typo);
    assert_eq!(typo.clone().merge_with(&none), typo);
}

/// A higher rank wins whatever leads; among equals the leading one stands.
#[test]
fn a_real_number_replaces_a_lower_one_whichever_leads_and_equals_keep_the_leading_one() {
    let apple = Isin::new("US0378331005").unwrap();
    let microsoft = Isin::new("US5949181045").unwrap();
    let provisional = closed("ZZ000A0B1C2");
    let other_provisional = closed("ZZ000000001");
    let dsb = closed("EZ1234567AB");
    let typo = open("US037833100");
    let masked = Isin::new("XX0000000001").unwrap();
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
    // A typo under a listed prefix yields to the real number in either
    // order, and a masked number - closing nowhere, listed nowhere - yields
    // to a typo, to a provisional number and to a real one.
    for (lower, higher) in [
        (&typo, &apple),
        (&masked, &typo),
        (&masked, &provisional),
        (&masked, &apple),
    ] {
        assert_eq!(lower.clone().merge_with(higher), *higher);
        assert_eq!(higher.clone().merge_with(lower), *higher);
    }
    // Two of one rank: this one. A typo under a listed prefix and a closing
    // `ZZ` number each have one of the two readings, so neither leads the
    // other by rank and the order decides.
    let other_typo = open("US594918104");
    assert_eq!(typo.clone().merge_with(&other_typo), typo);
    assert_eq!(typo.clone().merge_with(&provisional), typo);
    assert_eq!(provisional.clone().merge_with(&typo), provisional);
}

#[test]
fn intake_folds_the_case_and_a_column_holds_the_upper_case_shape() {
    let provisional = closed("ZZ000000001");
    assert!(Isin::is_canonical(provisional.as_str()));
    assert_eq!(
        Isin::new(provisional.as_str().to_ascii_lowercase()).unwrap(),
        provisional
    );
    // Canonical is the spelling a column holds - upper case, the shape -
    // and says nothing of the check digit, which is `closes`' question.
    assert!(Isin::is_canonical("XX0000000001"));
    assert!(!Isin::is_canonical("xx0000000001"));
    assert!(!Isin::is_canonical("XX000000000"));
    assert!(!Isin::is_canonical("US037833100A"));
}
