//! Resource identifier unit tests.

use super::{Authority, Scheme, Uri, UriPath, Url, Urn};

#[test]
fn canonical_uri_round_trip() {
    let uri = Uri::from_str("HTTPS://example.test/a%2fb?q=x#part").unwrap();
    assert_eq!(uri.to_string(), "https://example.test/a%2Fb?q=x#part");
    assert_eq!(Uri::from_str(&uri.to_string()).unwrap(), uri);
}

#[test]
fn components_validate_and_serialize_as_strings() {
    let scheme = Scheme::from_str("FILE").unwrap();
    let authority = Authority::from_str("").unwrap();
    let path = UriPath::from_str("/tmp/a.csv").unwrap();
    assert_eq!(serde_json::to_string(&scheme).unwrap(), "\"file\"");
    assert_eq!(authority.as_str(), "");
    assert_eq!(path.extension(), Some("csv"));
}

#[test]
fn specialized_values_validate() {
    assert!(Url::from_str("https://example.test").is_ok());
    assert!(Url::from_str("https:///path").is_err());
    let urn = Urn::from_str("URN:ISBN:9780131103627").unwrap();
    assert_eq!(urn.to_string(), "urn:isbn:9780131103627");
}

#[test]
fn authority_ports_are_explicit_valid_u16_values() {
    assert_eq!(
        Authority::from_str("minio:9000").unwrap().port(),
        Some(9000)
    );
    assert_eq!(
        Authority::from_str("[::1]:9000").unwrap().port(),
        Some(9000)
    );
    for invalid in ["minio:", "minio:65536", "[::1]:", "[::1]:99999"] {
        assert!(Authority::from_str(invalid).is_err(), "{invalid}");
    }
}

/// Percent-decoded access to the components that can carry escapes.
mod decoding {
    use super::{Uri, Url, Urn};

    #[test]
    fn a_component_answers_raw_or_as_the_text_its_escapes_stand_for() {
        let url = Url::from_str("https://example.com/a%20b/c%2Fd?q=a%26b#p%C3%A9").unwrap();

        assert_eq!(url.path().as_str(), "/a%20b/c%2Fd");
        assert_eq!(url.path_text(false).unwrap(), "/a%20b/c%2Fd");
        assert_eq!(url.query(false).unwrap().as_deref(), Some("q=a%26b"));
        assert_eq!(url.fragment(false).unwrap().as_deref(), Some("p%C3%A9"));

        // Decoding answers with text, not with structure: the encoded slash
        // stays inside the segment that carried it.
        assert_eq!(url.path_text(true).unwrap(), "/a b/c/d");
        assert_eq!(url.path_segments().collect::<Vec<_>>(), ["a%20b", "c%2Fd"]);
        assert_eq!(url.query(true).unwrap().as_deref(), Some("q=a&b"));
        assert_eq!(url.fragment(true).unwrap().as_deref(), Some("pé"));
    }

    #[test]
    fn text_with_nothing_to_decode_is_borrowed_rather_than_copied() {
        let uri = Uri::from_str("https://example.com/plain?q=1#top").unwrap();

        for decode in [false, true] {
            assert!(matches!(
                uri.path_text(decode).unwrap(),
                std::borrow::Cow::Borrowed(_)
            ));
            assert!(matches!(
                uri.query(decode).unwrap(),
                Some(std::borrow::Cow::Borrowed(_))
            ));
        }
    }

    #[test]
    fn an_escape_that_is_not_utf8_is_refused_rather_than_replaced() {
        let uri = Uri::from_str("https://example.com/a%FF?q=%FF").unwrap();

        assert!(uri.path_text(false).is_ok());
        let error = uri.path_text(true).unwrap_err().to_string();
        assert!(error.contains("UTF-8"), "{error}");
        assert!(uri.query(true).is_err());
    }

    #[test]
    fn a_urn_decodes_the_components_it_has() {
        let urn = Urn::from_str("urn:example:weather?=op=map%20view#now").unwrap();

        assert_eq!(urn.query(false).unwrap().as_deref(), Some("=op=map%20view"));
        assert_eq!(urn.query(true).unwrap().as_deref(), Some("=op=map view"));
        assert_eq!(urn.fragment(true).unwrap().as_deref(), Some("now"));
    }
}

/// The query addressed as its pairs.
mod parameters {
    use super::{Uri, Url};
    use crate::Parameters;

    fn url(value: &str) -> Url {
        Url::from_str(value).unwrap()
    }

    #[test]
    fn pairs_are_read_in_order_with_repeated_keys_kept() {
        let url = url("https://example.com/t?symbol=AAPL&venue=XNAS&symbol=MSFT&flag");
        let parameters = url.parameters(false).unwrap();

        assert_eq!(parameters.len(), 4);
        assert!(!parameters.is_empty());
        assert!(parameters.contains_key("flag"));
        assert_eq!(parameters.get("symbol"), Some("AAPL"));
        assert_eq!(
            parameters.get_all("symbol").collect::<Vec<_>>(),
            ["AAPL", "MSFT"]
        );
        // A pair carrying no `=` names an empty value rather than nothing.
        assert_eq!(parameters.get("flag"), Some(""));
        assert_eq!(parameters.get("absent"), None);
        assert_eq!(
            parameters.keys().collect::<Vec<_>>(),
            ["symbol", "venue", "symbol", "flag"]
        );
        assert_eq!(
            parameters.values().collect::<Vec<_>>(),
            ["AAPL", "XNAS", "MSFT", ""]
        );
        assert_eq!(
            parameters.iter().collect::<Vec<_>>(),
            [
                ("symbol", "AAPL"),
                ("venue", "XNAS"),
                ("symbol", "MSFT"),
                ("flag", "")
            ]
        );
    }

    #[test]
    fn a_missing_or_empty_query_reads_as_no_pairs() {
        for value in [
            "https://example.com/t",
            "https://example.com/t?",
            "https://example.com/t?&&",
        ] {
            let url = url(value);
            assert!(url.parameters(true).unwrap().is_empty(), "{value}");
        }
    }

    #[test]
    fn a_decoding_view_reads_text_and_a_raw_view_reads_the_query() {
        let url = url("https://example.com/t?as%20of=2026-01-02&note=a%26b");

        let raw = url.parameters(false).unwrap();
        assert_eq!(raw.get("as%20of"), Some("2026-01-02"));
        assert_eq!(raw.get("note"), Some("a%26b"));
        assert!(!raw.decoded());

        let decoded = url.parameters(true).unwrap();
        assert!(decoded.decoded());
        assert_eq!(decoded.get("as of"), Some("2026-01-02"));
        assert_eq!(decoded.get("note"), Some("a&b"));
    }

    #[test]
    fn editing_replaces_the_first_pair_and_drops_the_rest() {
        let mut parameters =
            Parameters::parse("symbol=AAPL&venue=XNAS&symbol=MSFT", false).unwrap();

        assert_eq!(
            parameters.insert("symbol", "TSLA").unwrap().as_deref(),
            Some("AAPL")
        );
        assert_eq!(
            parameters.iter().collect::<Vec<_>>(),
            [("symbol", "TSLA"), ("venue", "XNAS")]
        );

        parameters.append("symbol", "NVDA").unwrap();
        assert_eq!(
            parameters.get_all("symbol").collect::<Vec<_>>(),
            ["TSLA", "NVDA"]
        );

        // Removing takes every pair the key names, answering with the first.
        assert_eq!(parameters.remove("symbol").as_deref(), Some("TSLA"));
        assert_eq!(parameters.iter().collect::<Vec<_>>(), [("venue", "XNAS")]);
        assert_eq!(parameters.remove("symbol"), None);

        assert_eq!(parameters.insert("region", "us").unwrap(), None);
        assert_eq!(
            parameters.to_query().as_deref(),
            Some("venue=XNAS&region=us")
        );

        parameters.clear();
        assert_eq!(parameters.to_query(), None);
    }

    #[test]
    fn a_decoding_view_encodes_what_the_query_cannot_carry() {
        let mut url = url("https://example.com/trades");
        let mut parameters = url.parameters(true).unwrap().into_owned();
        parameters.insert("as of", "2026-01-02 09:30").unwrap();
        parameters.append("note", "a&b=c+d").unwrap();
        url.set_parameters(&parameters).unwrap();

        assert_eq!(
            url.to_string(),
            "https://example.com/trades?as%20of=2026-01-02%2009:30&note=a%26b%3Dc%2Bd"
        );
        let round_trip = url.parameters(true).unwrap();
        assert_eq!(round_trip.get("as of"), Some("2026-01-02 09:30"));
        assert_eq!(round_trip.get("note"), Some("a&b=c+d"));
    }

    #[test]
    fn a_raw_view_refuses_text_the_query_syntax_cannot_carry() {
        let mut parameters = Parameters::parse("symbol=AAPL", false).unwrap();

        for (key, value) in [("as of", "x"), ("note", "a&b"), ("note", "a=b")] {
            let error = parameters.insert(key, value).unwrap_err().to_string();
            assert!(!error.is_empty(), "{key}={value}");
        }
        assert_eq!(parameters.to_query().as_deref(), Some("symbol=AAPL"));

        // The same text is accepted by a decoding view, which encodes it.
        let mut decoded = Parameters::parse("symbol=AAPL", true).unwrap();
        decoded.insert("as of", "a&b").unwrap();
        assert_eq!(
            decoded.to_query().as_deref(),
            Some("symbol=AAPL&as%20of=a%26b")
        );
    }

    #[test]
    fn writing_pairs_back_replaces_only_the_query() {
        let mut url = url("https://example.com/a/b?symbol=AAPL#part");
        let mut parameters = url.parameters(false).unwrap().into_owned();
        parameters.insert("symbol", "MSFT").unwrap();
        url.set_parameters(&parameters).unwrap();
        assert_eq!(url.to_string(), "https://example.com/a/b?symbol=MSFT#part");

        // An empty view clears the component; setting it back restores it.
        url.set_parameters(&Parameters::parse("", false).unwrap())
            .unwrap();
        assert_eq!(url.to_string(), "https://example.com/a/b#part");
        assert!(url.query(false).unwrap().is_none());

        url.set_query(Some("symbol=NVDA")).unwrap();
        assert_eq!(url.query(false).unwrap().as_deref(), Some("symbol=NVDA"));

        // An invalid component leaves the URL exactly as it was.
        assert!(url.set_query(Some("a b")).is_err());
        assert_eq!(url.query(false).unwrap().as_deref(), Some("symbol=NVDA"));
    }

    #[test]
    fn a_looked_up_value_outlives_the_key_it_was_named_by() {
        let url = url("https://example.com/t?a=1&a=2");
        let parameters = url.parameters(false).unwrap();

        // The values borrow the view, so the name used to find them is free to
        // go out of scope first.
        let collected: Vec<&str> = {
            let key = String::from("a");
            parameters.get_all(&key).collect()
        };
        assert_eq!(collected, ["1", "2"]);
    }

    #[test]
    fn decoding_can_name_one_key_twice_where_the_raw_query_named_two() {
        let url = url("https://example.com/t?a%62=1&ab=2");

        // The raw view reads the query's own bytes, so the keys differ.
        assert_eq!(
            url.parameters(false).unwrap().iter().collect::<Vec<_>>(),
            [("a%62", "1"), ("ab", "2")]
        );

        // Decoded they are the same text, which is a repeated key: the first
        // answers, and an edit keeps one pair where the query held two.
        let decoded = url.parameters(true).unwrap();
        assert_eq!(
            decoded.iter().collect::<Vec<_>>(),
            [("ab", "1"), ("ab", "2")]
        );
        assert_eq!(decoded.get("ab"), Some("1"));

        let mut edited = decoded.into_owned();
        edited.insert("ab", "3").unwrap();
        assert_eq!(edited.to_query().as_deref(), Some("ab=3"));
    }

    #[test]
    fn an_encoded_value_survives_one_write_and_read_unchanged() {
        // Round trip through the query syntax: what a decoding view is given
        // is what the next decoding view answers with.
        for value in [
            "a&b",
            "a=b",
            "a+b",
            "100%",
            "%41",
            "a b",
            "é",
            "a/b?c:d@e;f,g",
        ] {
            let mut url = url("https://example.com/t");
            let mut parameters = url.parameters(true).unwrap().into_owned();
            parameters.append("k", value).unwrap();
            url.set_parameters(&parameters).unwrap();

            assert_eq!(
                url.parameters(true).unwrap().get("k"),
                Some(value),
                "{value}"
            );
        }
    }

    #[test]
    fn a_written_query_is_canonical_the_way_a_parsed_one_is() {
        let mut written = url("https://example.com/t");
        written.set_query(Some("note=a%c3%a9&as%20of=1")).unwrap();
        let parsed = url("https://example.com/t?note=a%c3%a9&as%20of=1");

        // Both spellings normalize their escapes, so the two values are one.
        assert_eq!(written.to_string(), parsed.to_string());
        assert_eq!(
            written.query(false).unwrap().as_deref(),
            Some("note=a%C3%A9&as%20of=1")
        );
        assert_eq!(written, parsed);
        assert_eq!(written.stable_hash(), parsed.stable_hash());

        // The same holds for a query written through the pair view.
        let mut edited = url("https://example.com/t");
        let mut parameters = edited.parameters(false).unwrap().into_owned();
        parameters.append("note", "a%c3%a9").unwrap();
        edited.set_parameters(&parameters).unwrap();
        assert_eq!(
            edited.query(false).unwrap().as_deref(),
            Some("note=a%C3%A9")
        );
        assert_eq!(edited.parameters(true).unwrap().get("note"), Some("aé"));
    }

    #[test]
    fn an_escape_that_is_not_utf8_refuses_the_decoding_view() {
        let uri = Uri::from_str("https://example.com/t?q=%FF").unwrap();

        assert!(uri.parameters(false).is_ok());
        assert!(uri.parameters(true).is_err());

        // An overlong encoding and a lone surrogate stand for no text either,
        // so neither can slip through as one.
        for query in ["q=%C0%AF", "q=%ED%A0%80"] {
            let uri = Uri::from_str(&format!("https://example.com/t?{query}")).unwrap();
            assert!(uri.parameters(true).is_err(), "{query}");
        }
    }
}
