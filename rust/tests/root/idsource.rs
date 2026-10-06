//! `rust/src/idsource.rs`: who gave an identifier - the source half of its
//! unique key `src:type` - a word folded to lower case, the sources the crate
//! names held as members and any other as an `Other` word.

use yggdryl::{IdKey, IdSource, IdType, Identifier, Identifiers};

fn source(text: &str) -> IdSource {
    text.parse().unwrap()
}

#[test]
fn a_source_folds_to_lower_case_and_reads_every_alias() {
    for (spelling, expected) in [
        ("base", IdSource::Base),
        ("BASE", IdSource::Base),
        (" Base ", IdSource::Base),
        ("DERIVED", IdSource::Derived),
        ("FIX", IdSource::Base),
        ("fix", IdSource::Base),
        ("BIC", IdSource::Bic),
        ("Proprietary", IdSource::Proprietary),
        ("ProprietaryCustomCode", IdSource::Proprietary),
        ("proprietary_custom_code", IdSource::Proprietary),
        ("GeneralIdentifier", IdSource::GeneralIdentifier),
        (
            "GenerallyAcceptedMarketParticipantIdentifier",
            IdSource::GeneralIdentifier,
        ),
        ("LegalEntityIdentifier", IdSource::LegalEntityIdentifier),
        ("Short Code Identifier", IdSource::ShortCodeIdentifier),
        ("MIC", IdSource::Mic),
        ("Tax-Id", IdSource::TaxId),
    ] {
        let read: IdSource = spelling.parse().unwrap();
        assert_eq!(read, expected, "{spelling:?}");
        assert!(read.is_known(), "{spelling:?}");
    }
    assert_eq!(IdSource::Base.as_str(), "base");
    assert_eq!(IdSource::Derived.as_str(), "derived");
    assert!(
        IdSource::KNOWN.iter().all(|known| known.as_str() != "fix"),
        "the standard's own fields are the base: fix is read and never written"
    );
    assert_eq!(IdSource::Proprietary.as_str(), "proprietary");
    assert_eq!(IdSource::GeneralIdentifier.as_str(), "generalidentifier");
    assert_eq!(
        IdSource::LegalEntityIdentifier.as_str(),
        "legalentityidentifier"
    );
    assert_eq!(
        IdSource::ShortCodeIdentifier.as_str(),
        "shortcodeidentifier"
    );
    assert_eq!(IdSource::Base.to_string(), "base");
}

#[test]
fn a_bridge_or_a_venue_is_an_other_word_and_unkn_no_longer_names_a_member() {
    // `oms` and `ullink` were members once: now a bridge is a word like any
    // venue's or firm's, and the base a source rests on is `base`.
    for word in ["oms", "OMS", "ullink", "ULLINK", "unkn", "venue", "firm.x"] {
        let read = source(word);
        assert!(!read.is_known(), "{word:?}");
        assert!(matches!(read, IdSource::Other(_)), "{word:?}");
        assert_eq!(read.as_str(), word.to_ascii_lowercase(), "{word:?}");
    }
    assert!(
        !IdSource::KNOWN
            .iter()
            .any(|known| matches!(known.as_str(), "oms" | "ullink" | "unkn"))
    );
    assert_ne!(source("oms"), IdSource::Base);
    assert_ne!(source("unkn"), IdSource::Base);
}

#[test]
fn any_other_word_is_kept_folded_and_is_not_a_member() {
    let firm = source("Firm.X");
    assert_eq!(firm.as_str(), "firm.x");
    assert!(!firm.is_known());
    assert_eq!(firm.to_string(), "firm.x");
    assert_eq!(firm, "firm.x");
    assert_eq!(source("Venue_1").as_str(), "venue1");
    assert_eq!(source("my firm").as_str(), "myfirm");
    assert_eq!(source(&"S".repeat(64)).as_str(), "s".repeat(64));
    // Two spellings of one word are one source, a member or not.
    assert_eq!(source("Firm.X"), source("firm.x"));
    assert_eq!(source("VENUE"), source("venue"));
    assert_ne!(source("firm"), IdSource::Base);
    // A source and a type are two vocabularies: the same word is each in its
    // own, and a source never reads as a type.
    assert!(source("isin").as_str() == IdType::Isin.as_str());
    assert!(!source("isin").is_known());
    assert!(IdType::Isin.is_known());
}

#[test]
fn a_spelling_that_is_no_word_is_refused_and_names_what_was_expected() {
    for refused in [
        "",
        "   ",
        "_-#",
        "caf\u{e9}",
        "tab\tsrc",
        "a/b",
        "a:b",
        "a=b",
    ] {
        assert!(refused.parse::<IdSource>().is_err(), "{refused:?}");
    }
    let refused = "".parse::<IdSource>().unwrap_err().to_string();
    assert!(
        refused.contains("an identifier source of a word"),
        "{refused}"
    );
    let refused = "caf\u{e9}".parse::<IdSource>().unwrap_err().to_string();
    assert!(
        refused.contains("ASCII letters, digits and '.'"),
        "{refused}"
    );
    let refused = "s".repeat(65).parse::<IdSource>().unwrap_err().to_string();
    assert!(refused.contains("at most 64 bytes"), "{refused}");
}

#[test]
fn sources_compare_with_text_and_order_by_their_spelling() {
    assert_eq!(IdSource::Base, "base");
    assert_eq!(IdSource::Base, *"base");
    assert!(IdSource::Base != "BASE", "a spelling is compared as folded");
    assert!(
        IdSource::Base != "fix",
        "fix reads as the base and is never its spelling"
    );
    let mut sorted = [
        IdSource::Base,
        source("venue"),
        IdSource::Derived,
        source("zzz"),
        IdSource::Bic,
        source("oms"),
    ];
    sorted.sort();
    assert_eq!(
        sorted.iter().map(IdSource::as_str).collect::<Vec<_>>(),
        ["base", "bic", "derived", "oms", "venue", "zzz"]
    );
    assert_eq!(AsRef::<str>::as_ref(&IdSource::Bic), "bic");
    assert_eq!(smol_str::SmolStr::from(IdSource::Derived), "derived");
    assert_eq!(smol_str::SmolStr::from(source("firm")), "firm");
}

#[test]
fn the_known_sources_are_unique_folded_and_read_back_as_themselves() {
    let mut seen = std::collections::HashSet::new();
    for known in &IdSource::KNOWN {
        assert!(known.is_known(), "{known}");
        let spelling = known.as_str();
        assert_eq!(
            spelling,
            spelling.to_ascii_lowercase(),
            "{spelling} is folded"
        );
        assert_eq!(&source(spelling), known, "{spelling} reads as itself");
        assert_eq!(
            &source(&spelling.to_ascii_uppercase()),
            known,
            "{spelling} reads upper case"
        );
        assert!(seen.insert(spelling), "{spelling} is named once");
    }
    assert_eq!(
        IdSource::KNOWN[0],
        IdSource::Base,
        "nothing names the source is first"
    );
    for named in [
        IdSource::Base,
        IdSource::Derived,
        IdSource::Proprietary,
        IdSource::Bic,
    ] {
        assert!(IdSource::KNOWN.contains(&named), "{named}");
    }
}

#[test]
fn a_source_is_the_first_half_of_an_identifiers_key_and_the_base_source_is_never_spelled() {
    let isin =
        |src: IdSource, value: &str| Identifier::new(IdKey::new(src, IdType::Isin), value).unwrap();
    let cusip = |src: IdSource, value: &str| {
        Identifier::new(IdKey::new(src, IdType::Cusip), value).unwrap()
    };
    let venue = source("venue");
    let mut ids = Identifiers::new();
    assert!(ids.insert(isin(IdSource::Base, "US0378331005")));
    assert!(ids.insert(isin(venue.clone(), "US0378331005")));
    assert!(ids.insert(cusip(IdSource::Derived, "037833100")));
    assert!(ids.insert(cusip(source("firm"), "037833100")));
    assert!(
        !ids.insert(isin(IdSource::Base, "CH0012214059")),
        "one value per key"
    );
    assert_eq!(
        ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "cusip=037833100",
            "firm:cusip=037833100",
            "isin=US0378331005",
            "venue:isin=US0378331005",
        ],
        "sorted by the key as spelled, the base source never spelled; the \
         firm's statement took the derivation back"
    );
    assert_eq!(
        ids.get_from(&IdKey::new(venue, IdType::Isin)),
        Some("US0378331005")
    );
    assert_eq!(
        ids.get_from(&IdKey::new(IdSource::Proprietary, IdType::Isin)),
        None
    );
    // The source is the key's first half: the same type under two sources
    // is two identifiers, and one source under two types is two as well.
    assert_eq!(ids.of_kind(&IdType::Isin).count(), 2);
    assert_eq!(ids.of_kind(&IdType::Cusip).count(), 2);
    assert!(
        ids.remove(&IdKey::base(IdType::Cusip)).is_some(),
        "the base key removes its type"
    );
    assert_eq!(ids.of_kind(&IdType::Cusip).count(), 0);
    assert!(ids.remove(&IdKey::base(IdType::Isin)).is_some());
    assert!(ids.remove(&IdKey::base(IdType::Isin)).is_none());
    assert!(ids.is_empty());
}

/// The source owns one rule beside its type's: every value `bic` gives is a
/// BIC and every value `legalentityidentifier` gives an LEI, held by the
/// code's shape and upper-cased; every other source the crate names - and
/// any other word - holds a value to its type's rule alone.
#[test]
fn only_the_bic_and_lei_sources_hold_their_values_to_a_code() {
    let held = |src: &IdSource, value: &str| {
        Identifier::new(IdKey::new(src.clone(), IdType::ExecutingFirm), value)
            .map(|id| id.value().to_owned())
    };
    for src in IdSource::KNOWN.iter().chain([&source("firm.x")]) {
        match src {
            IdSource::Bic => {
                assert!(held(src, "T-1").is_err());
                assert_eq!(held(src, "deutdeff").unwrap(), "DEUTDEFF");
                assert!(
                    held(src, "HWUPKR0MPOU8FGXBT394").is_err(),
                    "an LEI is no BIC"
                );
            }
            IdSource::LegalEntityIdentifier => {
                assert!(held(src, "T-1").is_err());
                assert_eq!(
                    held(src, "hwupkr0mpou8fgxbt394").unwrap(),
                    "HWUPKR0MPOU8FGXBT394"
                );
                assert!(held(src, "DEUTDEFF").is_err(), "a BIC is no LEI");
            }
            other => {
                assert_eq!(held(other, "t-1").unwrap(), "t-1", "{other}");
                assert_eq!(held(other, "deutdeff").unwrap(), "deutdeff", "{other}");
            }
        }
    }
}
