//! `rust/src/eusipa.rs`: the EUSIPA product category of a structured
//! product - a four-digit code held by its shape, named by the European
//! Derivative Map of February 2024 and by the SSPA Swiss Derivative Map of
//! 2023 and 2026 where each lists it.

use yggdryl::Eusipa;

fn code(value: u16) -> Eusipa {
    Eusipa::new(value).unwrap()
}

#[test]
fn a_code_that_is_no_four_digit_investment_or_leverage_category_is_refused() {
    // A category is a value, no datatype: its refusal is a value's, at the
    // value itself.
    for refused in [0, 7, 999, 3000, 3100, 9999, 10_000, u16::MAX] {
        let error = Eusipa::new(refused).unwrap_err();
        assert!(
            matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$"),
            "{refused}: {error:?}"
        );
        let error = error.to_string();
        assert!(
            error.contains("EUSIPA product category"),
            "{refused}: {error}"
        );
        assert!(error.contains(&refused.to_string()), "{refused}: {error}");
        assert!(!error.contains("datatype"), "{refused}: {error}");
    }
    assert_eq!(
        Eusipa::new(3100).unwrap_err().to_string(),
        "invalid record value at $: expected a four-digit EUSIPA product category opening with \
         1, an investment product, or 2, a leverage product, got 3100"
    );
    for text in [
        "",
        " ",
        "22",
        "230",
        "23000",
        "02300",
        "+2300",
        "-2300",
        "2300.0",
        "23 00",
        "abcd",
        "２３００",
    ] {
        assert!(text.parse::<Eusipa>().is_err(), "{text:?}");
        assert!(Eusipa::from_text(text).is_err(), "{text:?}");
    }
    assert_eq!(
        Eusipa::from_text("23x0").unwrap_err().to_string(),
        "invalid record value at $: expected a four-digit EUSIPA product category, got \"23x0\""
    );
    assert!(
        Eusipa::from_text("3100").is_err(),
        "four digits of no level"
    );
    assert!(Eusipa::try_from(999_u16).is_err());
}

#[test]
fn a_code_of_its_shape_is_held_whatever_either_map_lists() {
    // The maps are snapshots of lists that evolve: a code of the shape no
    // map lists - a member added later, or one retired - is a code.
    for held in [1000, 1110, 1999, 2000, 2301, 2999] {
        let category = code(held);
        assert_eq!(category.code(), held);
        assert!(!category.is_listed(), "{held}");
        assert_eq!(category.name(), None, "{held}");
        assert_eq!(category.sspa_name(), None, "{held}");
    }
    let constant = code(2300);
    assert_eq!(constant.code(), 2300);
    assert_eq!(constant.group(), 23);
    assert_eq!(constant.level(), 2);
    assert_eq!(code(1260).group(), 12);
    assert_eq!(code(1260).level(), 1);
    assert_eq!(u16::from(constant), 2300);
    assert_eq!(Eusipa::try_from(2300_u16).unwrap(), constant);
}

#[test]
fn a_code_reads_as_four_digits_trimmed_and_displays_as_them() {
    assert_eq!(" 2300 ".parse::<Eusipa>().unwrap(), code(2300));
    assert_eq!(Eusipa::from_text("1100").unwrap(), code(1100));
    assert_eq!(code(2300).to_string(), "2300");
    assert_eq!(code(1000).to_string(), "1000");
    for held in [1100, 1260, 2205, 2399] {
        assert_eq!(
            code(held).to_string().parse::<Eusipa>().unwrap(),
            code(held)
        );
    }
    assert!(code(1100) < code(2300), "codes order by number");
}

#[test]
fn a_code_crosses_serde_as_its_number_and_a_number_of_no_shape_is_refused() {
    assert_eq!(serde_json::to_string(&code(2300)).unwrap(), "2300");
    assert_eq!(serde_json::from_str::<Eusipa>("2300").unwrap(), code(2300));
    assert!(serde_json::from_str::<Eusipa>("3100").is_err());
    assert!(serde_json::from_str::<Eusipa>("\"2300\"").is_err());
}

/// Every member of the European Derivative Map of February 2024, by its
/// English name; the two credit-linked tranches continue the line above
/// them, spelled out.
#[test]
fn the_european_map_names_every_member_it_lists() {
    let eusipa = [
        (1100, "Uncapped Capital Protection"),
        (1120, "Capped Capital Protection"),
        (1130, "Capital Protection with Knock-Out"),
        (1140, "Capital Protection with Coupon"),
        (1199, "Miscellaneous Capital Protection"),
        (1200, "Discount Certificates"),
        (1210, "Barrier Discount Certificates"),
        (1220, "Reverse Convertibles"),
        (1230, "Barrier Reverse Convertibles"),
        (1240, "Capped Outperformance Certificates"),
        (1250, "Capped Bonus Certificates"),
        (1260, "Express Certificates"),
        (1299, "Miscellaneous Yield Enhancement"),
        (1300, "Tracker Certificates"),
        (1310, "Outperformance Certificates"),
        (1320, "Bonus Certificates"),
        (1330, "Outperformance Bonus Certificates"),
        (1340, "Twin-Win Certificates"),
        (1399, "Miscellaneous Participation"),
        (1440, "Credit Linked Note - Linear"),
        (1450, "Credit Linked Note - Equity Tranche"),
        (1460, "Credit Linked Note - Mezz./Senior Tranche"),
        (1499, "Miscellaneous Credit Linked Notes"),
        (2100, "Warrants"),
        (2110, "Spread Warrants"),
        (2199, "Miscellaneous"),
        (2200, "Knock-Out Warrants"),
        (2205, "Open-end Knock-Out Warrants"),
        (2210, "Mini-Futures"),
        (2230, "Double Knock-Out Warrants"),
        (2299, "Miscellaneous"),
        (2300, "Constant Leverage Certificate"),
        (2399, "Miscellaneous Constant Leverage Products"),
    ];
    for (held, name) in eusipa {
        assert_eq!(code(held).name(), Some(name), "{held}");
        assert!(code(held).is_listed(), "{held}");
    }
    let listed = (1000..=2999)
        .filter(|held| code(*held).name().is_some())
        .count();
    assert_eq!(listed, eusipa.len(), "no member past the map");
}

/// Every member of the SSPA Swiss Derivative Map, whose 2023 and 2026
/// category lists are one list.
#[test]
fn the_swiss_map_names_every_member_it_lists() {
    let sspa = [
        (1100, "Capital Protection Note with Participation"),
        (1130, "Capital Protection Note with Barrier"),
        (1135, "Capital Protection Note with Twin Win"),
        (1140, "Capital Protection Note with Coupon"),
        (1200, "Discount Certificate"),
        (1210, "Barrier Discount Certificate"),
        (1220, "Reverse Convertible"),
        (1230, "Barrier Reverse Convertible"),
        (1255, "Conditional Coupon Reverse Convertible"),
        (1260, "Conditional Coupon Barrier Reverse Convertible"),
        (1300, "Tracker Certificate"),
        (1310, "Outperformance Certificate"),
        (1320, "Bonus Certificate"),
        (1330, "Bonus Outperformance Certificate"),
        (1340, "Twin Win Certificate"),
        (1400, "Credit Linked Notes"),
        (
            1410,
            "Conditional Capital Protection Note with add. credit risk",
        ),
        (1420, "Yield Enhancement Certificate with add. credit risk"),
        (1430, "Participation Certificate with add. credit risk"),
        (2100, "Warrant"),
        (2110, "Spread Warrant"),
        (2200, "Warrant with Knock-Out"),
        (2210, "Mini-Future"),
        (2300, "Constant Leverage Certificate"),
    ];
    for (held, name) in sspa {
        assert_eq!(code(held).sspa_name(), Some(name), "{held}");
        assert!(code(held).is_listed(), "{held}");
    }
    let listed = (1000..=2999)
        .filter(|held| code(*held).sspa_name().is_some())
        .count();
    assert_eq!(listed, sspa.len(), "no member past the map");
}

/// The maps share their numbering but not every member: one lists what the
/// other does not, and 1260 is the one code they name apart - an Express
/// Certificate in Europe, a Conditional Coupon Barrier Reverse Convertible
/// in Switzerland.
#[test]
fn the_two_maps_disagree_on_1260_and_each_lists_members_the_other_lacks() {
    let express = code(1260);
    assert_eq!(express.name(), Some("Express Certificates"));
    assert_eq!(
        express.sspa_name(),
        Some("Conditional Coupon Barrier Reverse Convertible")
    );
    for swiss_only in [1135, 1255, 1400, 1410, 1420, 1430] {
        assert_eq!(code(swiss_only).name(), None, "{swiss_only}");
        assert!(code(swiss_only).sspa_name().is_some(), "{swiss_only}");
        assert!(code(swiss_only).is_listed(), "{swiss_only}");
    }
    for european_only in [1120, 1199, 1240, 1250, 1440, 1450, 1460, 2205, 2230, 2399] {
        assert!(code(european_only).name().is_some(), "{european_only}");
        assert_eq!(code(european_only).sspa_name(), None, "{european_only}");
    }
    // The worked example: one name in both.
    assert_eq!(code(2300).name(), code(2300).sspa_name());
}
