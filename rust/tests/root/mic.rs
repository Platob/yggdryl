//! `rust/src/mic.rs`: ISO 10383's market identifier code, and what its
//! registry states of each code - the operating MIC, whether it is a
//! segment, the country.

use yggdryl::{Country, Mic};

fn mic(text: &str) -> Mic {
    Mic::new(text).unwrap()
}

/// A segment answers the operating MIC of its market and an operating MIC
/// itself; an expired segment still answers, because a capture names venues
/// that have since closed.
#[test]
fn a_segment_answers_its_market_and_an_operating_mic_itself() {
    let segment = mic("XNGS");
    assert_eq!(segment.operating(), Some(mic("XNAS")));
    assert!(segment.is_segment());
    for operating in ["XNAS", "XLON", "XETR", "XSWX", "XXXX"] {
        let held = mic(operating);
        assert_eq!(held.operating(), Some(held.clone()), "{operating}");
        assert!(!held.is_segment(), "{operating}");
    }
    // `NBXO` expired under `XBXO`, which has since become a segment of
    // `XNAS`: the chain ends at the operating MIC.
    let expired = mic("NBXO");
    assert_eq!(expired.operating(), Some(mic("XNAS")));
    assert!(expired.is_segment());
}

/// A code answers the country ISO 10383 places it in; a venue of no single
/// country - the registry's own `ZZ` - answers none, as a code the registry
/// never assigned does.
#[test]
fn a_code_answers_the_country_its_registry_places_it_in() {
    for (code, country) in [
        ("XLON", "GB"),
        ("XETR", "DE"),
        ("XSWX", "CH"),
        ("XNAS", "US"),
        ("XNGS", "US"),
    ] {
        assert_eq!(
            mic(code).country(),
            Some(Country::new(country).unwrap()),
            "{code}"
        );
    }
    for nowhere in ["XOFF", "XXXX"] {
        assert_eq!(mic(nowhere).country(), None, "{nowhere}");
    }
}

/// A code the registry never assigned - a venue's own short code, a
/// lower-case spelling - answers no operating MIC, no segment and no
/// country.
#[test]
fn a_code_the_registry_never_assigned_answers_nothing() {
    for unknown in ["QQQQ", "xnas", "S", ""] {
        let held = mic(unknown);
        assert_eq!(held.operating(), None, "{unknown}");
        assert!(!held.is_segment(), "{unknown}");
        assert_eq!(held.country(), None, "{unknown}");
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::mic::mics;
    use yggdryl::{Country, Mic, StringEnum};

    use super::mic;

    /// The generated table is sorted by the MIC with each code once - what
    /// the binary search needs - every MIC and operating MIC four upper-case
    /// letters or digits, every operating MIC a row naming itself, and every
    /// row what the three accessors answer; every common venue the logical
    /// `mic` enum lists is a registered code.
    #[test]
    fn the_generated_registry_table_is_sorted_unique_and_what_the_accessors_answer() {
        let rows = mics();
        assert!(
            rows.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "sorted by the MIC, each code once"
        );
        for &(code, operating, country) in rows {
            assert!(Mic::is_iso(code), "{code}");
            assert!(Mic::is_iso(operating), "{code}: {operating}");
            assert!(
                country.is_empty()
                    || (country.len() == 2
                        && country.bytes().all(|byte| byte.is_ascii_uppercase())),
                "{code}: {country}"
            );
            let held = mic(code);
            assert_eq!(held.operating(), Some(mic(operating)), "{code}");
            assert_eq!(held.is_segment(), code != operating, "{code}");
            // The row's country, where it is a listed one: `ZZ` is none.
            let listed = Country::new(country).unwrap();
            assert_eq!(
                held.country(),
                listed.is_listed().then_some(listed),
                "{code}"
            );
            assert_eq!(
                mic(operating).operating(),
                Some(mic(operating)),
                "{code}: {operating}"
            );
        }
        for code in StringEnum::MICS {
            assert!(mic(code).operating().is_some(), "{code}");
        }
    }
}
