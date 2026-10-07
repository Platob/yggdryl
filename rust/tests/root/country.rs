//! `rust/src/country.rs`: ISO 3166-1's two-letter country code, held by
//! its shape and ranked by the listing.

use yggdryl::{Ccy, CodeValue, Country, DataType, Scalar, StringEnum};

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

/// A country answers the one legal tender ISO 4217 list one gives it: a
/// fund code never, and where list one gives two tenders the one the
/// generator's override table names.
#[test]
fn a_country_answers_the_one_legal_tender_list_one_gives_it() {
    for (country, currency) in [
        ("US", "USD"),
        ("CH", "CHF"),
        ("DE", "EUR"),
        ("GB", "GBP"),
        ("JP", "JPY"),
        // A territory using another country's tender.
        ("LI", "CHF"),
        ("AX", "EUR"),
        // Two tenders in list one: the override table's choice.
        ("SV", "USD"),
        ("PA", "PAB"),
        ("LS", "LSL"),
        ("VE", "VES"),
    ] {
        assert_eq!(
            Country::new(country).unwrap().currency(),
            Some(Ccy::new(currency).unwrap()),
            "{country}"
        );
    }
    // None where list one gives no currency: the user-assigned codes, an
    // agency prefix, a country with no universal currency, a spelling no
    // code is.
    for none in ["XX", "ZZ", "XS", "EU", "AQ", "PS", "GS", "ch", ""] {
        assert_eq!(Country::new(none).unwrap().currency(), None, "{none}");
    }
    // Every listed country has a tender but the three list one gives no
    // universal currency.
    let unanswered: Vec<&str> = StringEnum::COUNTRIES
        .iter()
        .copied()
        .filter(|code| Country::new(code).unwrap().currency().is_none())
        .collect();
    assert_eq!(unanswered, ["AQ", "GS", "PS"]);
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::country::country_currency;
    use yggdryl::{Ccy, Country, StringEnum};

    /// The generated table is sorted by the alpha-2 code with each code once,
    /// what the binary search needs; every code is listed, every currency is
    /// three upper-case letters ISO 4217 lists, and every row is what
    /// `currency` answers.
    #[test]
    fn the_generated_currency_table_is_sorted_unique_and_what_currency_answers() {
        let rows = country_currency();
        assert!(
            rows.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "sorted by the alpha-2 code, each code once"
        );
        for &(country, currency) in rows {
            let code = Country::new(country).unwrap();
            assert!(code.is_listed(), "{country}");
            assert!(
                currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase()),
                "{country}: {currency}"
            );
            assert!(
                StringEnum::CURRENCIES.contains(&currency),
                "{country}: {currency}"
            );
            assert_eq!(
                code.currency(),
                Some(Ccy::new(currency).unwrap()),
                "{country}"
            );
        }
    }
}
