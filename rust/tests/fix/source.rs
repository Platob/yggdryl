//! `rust/src/fix/source.rs`: one source a dictionary was built from - the
//! entry a `FIX:sources` id names - its id grammar and its stored entry.

use yggdryl::{Error, FixSource, PluginSide};

#[test]
fn an_id_is_a_word_folded_to_ascii_lowercase() {
    let source = FixSource::new("AUTEX_FIX42").unwrap();
    assert_eq!(source.id(), "autex_fix42");
    assert_eq!(source.file(), None);
    // A comma, a space, a dot and a leading digit are ordinary characters of
    // an id: the stored array is what separates the ids.
    assert_eq!(FixSource::new("ms,bloomberg").unwrap().id(), "ms,bloomberg");
    assert_eq!(
        FixSource::new("Morgan Stanley").unwrap().id(),
        "morgan stanley"
    );
    assert_eq!(FixSource::new("4.4-ms").unwrap().id(), "4.4-ms");
    // What the stored array would have to escape is no id, and the refusal
    // names the key a field states ids under.
    for refused in ["", "a\"b", "a\\b", "a\u{1}b", "a\nb"] {
        let error = FixSource::new(refused).unwrap_err();
        assert!(
            matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:sources"),
            "{refused:?}: {error}"
        );
        assert!(error.to_string().contains("non-empty source id"), "{error}");
        assert!(
            error.to_string().contains(&format!("{refused:?}")),
            "{error}"
        );
    }
}

#[test]
fn an_entry_states_its_file_where_one_is_known_and_round_trips_as_json() {
    let bare = FixSource::new("venue").unwrap();
    let named = FixSource::new("venue").unwrap().with_file("Venue.cfb");
    assert_eq!(named.id(), "venue");
    assert_eq!(named.file(), Some("Venue.cfb"), "kept as the file is named");
    assert_ne!(bare, named);
    // The entry a store writes: the file left out where none is known,
    // the plugin side always stated.
    assert_eq!(
        yggdryl::into_json_scalar(&bare.into_scalar()).unwrap(),
        r#"{"id":"venue","pluginside":"UKNW"}"#
    );
    assert_eq!(
        yggdryl::into_json_scalar(&named.into_scalar()).unwrap(),
        r#"{"file":"Venue.cfb","id":"venue","pluginside":"UKNW"}"#
    );
    for source in [&bare, &named] {
        assert_eq!(
            FixSource::from_scalar(&source.into_scalar()).unwrap(),
            *source
        );
    }
    // Read as a file spells it: the id folded, the file kept as spelled.
    let spelled = yggdryl::from_json_scalar(r#"{"id": "VENUE", "file": "Venue.cfb"}"#).unwrap();
    assert_eq!(FixSource::from_scalar(&spelled).unwrap(), named);
}

#[test]
fn a_stored_entry_that_is_not_one_is_refused_by_what_it_lacks() {
    for (document, expected) in [
        ("[]", "a JSON source entry object"),
        (r#"{"file": "x.cfb"}"#, "stating its id as text"),
        (r#"{"id": 7}"#, "stating its id as text"),
        (
            r#"{"id": "venue", "file": 7}"#,
            "the file of \"venue\" as text",
        ),
        (
            r#"{"id": "venue", "url": "x"}"#,
            "expected the keys \"id\", \"file\" and \"pluginside\", got \"url\"",
        ),
    ] {
        let value = yggdryl::from_json_scalar(document).unwrap();
        let error = FixSource::from_scalar(&value).unwrap_err();
        assert!(
            matches!(error, Error::InvalidRecord { .. }),
            "{document}: {error}"
        );
        assert!(error.to_string().contains(expected), "{document}: {error}");
    }
    // An id that is no id is the id grammar's refusal, as at every door.
    let value = yggdryl::from_json_scalar(r#"{"id": ""}"#).unwrap();
    let error = FixSource::from_scalar(&value).unwrap_err();
    assert!(
        matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:sources"),
        "{error}"
    );
}

#[test]
fn entries_order_by_id_then_by_file() {
    let mut held = [
        FixSource::new("b").unwrap(),
        FixSource::new("a").unwrap().with_file("a.cfb"),
        FixSource::new("a").unwrap(),
    ];
    held.sort();
    assert_eq!(
        held.iter()
            .map(|source| (source.id(), source.file()))
            .collect::<Vec<_>>(),
        [("a", None), ("a", Some("a.cfb")), ("b", None)]
    );
}

/// An entry states the role of its plugin - what a CBlock's root `type`
/// names - `UKNW` where none is known, and reads it back from the stored
/// name in any spelling the enum reads or from its code; absent is `UKNW`,
/// and a value naming no member is refused naming the id.
#[test]
fn an_entry_states_the_role_of_its_plugin_and_absent_reads_as_none() {
    let bare = FixSource::new("venue").unwrap();
    assert_eq!(bare.pluginside(), PluginSide::Unknown);
    let sell = FixSource::new("venue")
        .unwrap()
        .with_file("Venue.cfb")
        .with_pluginside(PluginSide::SellSide);
    assert_eq!(sell.pluginside(), PluginSide::SellSide);
    assert_ne!(bare, sell);
    assert_eq!(
        yggdryl::into_json_scalar(&sell.into_scalar()).unwrap(),
        r#"{"file":"Venue.cfb","id":"venue","pluginside":"SELL"}"#
    );
    assert_eq!(FixSource::from_scalar(&sell.into_scalar()).unwrap(), sell);
    for (document, expected) in [
        (
            r#"{"id": "venue", "file": "Venue.cfb"}"#,
            PluginSide::Unknown,
        ),
        (
            r#"{"id": "venue", "file": "Venue.cfb", "pluginside": "SELL"}"#,
            PluginSide::SellSide,
        ),
        (
            r#"{"id": "venue", "file": "Venue.cfb", "pluginside": "buy-side"}"#,
            PluginSide::BuySide,
        ),
        (
            r#"{"id": "venue", "file": "Venue.cfb", "pluginside": "uknw"}"#,
            PluginSide::Unknown,
        ),
        (
            r#"{"id": "venue", "file": "Venue.cfb", "pluginside": 2}"#,
            PluginSide::SellSide,
        ),
    ] {
        let value = yggdryl::from_json_scalar(document).unwrap();
        let source = FixSource::from_scalar(&value).unwrap();
        assert_eq!(source.pluginside(), expected, "{document}");
        assert_eq!(source.file(), Some("Venue.cfb"), "{document}");
    }
    for (document, expected) in [
        (
            r#"{"id": "venue", "pluginside": "X"}"#,
            "as a member - BUYS, SELL or UKNW - got \"X\"",
        ),
        (r#"{"id": "venue", "pluginside": "UNKN"}"#, "got \"UNKN\""),
        (r#"{"id": "venue", "pluginside": 7}"#, "got 7"),
        (r#"{"id": "venue", "pluginside": -1}"#, "got -1"),
        (r#"{"id": "venue", "pluginside": null}"#, "got null"),
    ] {
        let value = yggdryl::from_json_scalar(document).unwrap();
        let error = FixSource::from_scalar(&value).unwrap_err();
        assert!(
            matches!(error, Error::InvalidRecord { .. }),
            "{document}: {error}"
        );
        let text = error.to_string();
        assert!(
            text.contains("the pluginside of \"venue\""),
            "{document}: {text}"
        );
        assert!(text.contains(expected), "{document}: {text}");
    }
    // The role orders an entry after its id and its file.
    let mut held = [
        FixSource::new("a")
            .unwrap()
            .with_pluginside(PluginSide::SellSide),
        FixSource::new("a").unwrap(),
        FixSource::new("a")
            .unwrap()
            .with_pluginside(PluginSide::BuySide),
    ];
    held.sort();
    assert_eq!(
        held.iter().map(FixSource::pluginside).collect::<Vec<_>>(),
        [
            PluginSide::Unknown,
            PluginSide::BuySide,
            PluginSide::SellSide
        ]
    );
}
