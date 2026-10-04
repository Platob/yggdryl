//! `rust/src/country.rs`: ISO 3166-1's two-letter country code, held by
//! its shape and ranked by the listing.

use yggdryl::{CodeValue, Country, DataType, Scalar, StringEnum};

#[test]
fn a_country_is_at_most_two_ascii_bytes_and_the_listing_is_a_rank() {
    assert_eq!(<Country as CodeValue>::WIDTH, 2);
    assert_eq!(DataType::Country.code_width(), Some(2));
    let listed = Country::new("CH").unwrap();
    assert!(listed.is_listed());
    assert_eq!(listed.rank(), 1);
    assert!(listed.is_real());
    assert_eq!(<Country as CodeValue>::MAX_RANK, 1);
    // A code the registry does not assign - the user-assigned `XX`, the
    // transitional `AN` - is a value of rank zero, never a refusal.
    for unlisted in ["XX", "AN", "ZZ", ""] {
        let held = Country::new(unlisted).unwrap();
        assert_eq!(held.as_str(), unlisted);
        assert!(!held.is_listed(), "{unlisted}");
        assert_eq!(held.rank(), 0, "{unlisted}");
        assert_eq!(
            DataType::Country.scalar(unlisted).unwrap(),
            Scalar::from(held),
            "{unlisted}"
        );
    }
    // Every listed code ranks one, and the listing is the sorted one.
    for code in StringEnum::COUNTRIES {
        assert!(Country::new(code).unwrap().is_listed(), "{code}");
    }
    // The width is the refusal.
    let refused = Country::new("CHE").unwrap_err().to_string();
    assert!(refused.contains("at most 2 bytes"), "{refused}");
}

/// A listed country replaces an unlisted one whichever leads; two listed
/// ones keep the leading one.
#[test]
fn a_listed_country_replaces_an_unlisted_one_whichever_leads() {
    let listed = Country::new("CH").unwrap();
    let other = Country::new("US").unwrap();
    let masked = Country::new("XX").unwrap();
    let other_masked = Country::new("ZZ").unwrap();
    assert_eq!(masked.clone().merge_with(&listed), listed);
    assert_eq!(listed.clone().merge_with(&masked), listed);
    assert_eq!(listed.clone().merge_with(&other), listed);
    assert_eq!(other.clone().merge_with(&listed), other);
    assert_eq!(masked.clone().merge_with(&other_masked), masked);
}
