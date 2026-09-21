//! `rust/src/uri/parameters.rs`: the `key=value` pairs a query spells, read,
//! edited and written back as one component.

mod encoding {

    use yggdryl::{Error, Uri, Url};

    /// Return the target, byte offset, and reason a parse failure reports.
    fn parse_failure(error: &Error) -> (&str, usize, &str) {
        match error {
            Error::Parse {
                target,
                position,
                reason,
            } => (target, *position, reason.as_str()),
            other => panic!("expected a parse error, got {other}"),
        }
    }

    /// A query pair encodes the three bytes that would end it, `+` included.
    ///
    /// RFC 3986 reads `+` as a literal plus and HTML form decoding reads it as a
    /// space; a value that survives both is spelled `%2B`. Reading and writing are
    /// each other's inverse, which the S3 `+`-in-a-key bug class is what happens
    /// without.
    #[test]
    fn query_pairs_encode_what_would_end_them_and_read_back_the_same() {
        for query in [
            "a", "a=", "=b", "a=b=c", "a&&b", "=", "%3D=%26", "a=b+c", "a%3Db=c", "a=%2B", "a=%25",
        ] {
            let url = Url::from_str(&format!("https://example.test/?{query}")).unwrap();
            let pairs: Vec<(String, String)> = url
                .parameters(true)
                .unwrap()
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect();

            let mut written = url.clone();
            written
                .set_parameters(&url.parameters(true).unwrap().into_owned())
                .unwrap();
            let read_back: Vec<(String, String)> = written
                .parameters(true)
                .unwrap()
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect();
            assert_eq!(pairs, read_back, "{query:?} is not a fixed point");
        }

        let mut url = Url::from_str("https://example.test/t").unwrap();
        let mut parameters = url.parameters(true).unwrap().into_owned();
        parameters.append("as of", "2026-01-02 09:30").unwrap();
        parameters.append("note", "a&b=c+d%e").unwrap();
        parameters.append("100%", "\u{e9}").unwrap();
        url.set_parameters(&parameters).unwrap();

        assert_eq!(
            url.query(false).unwrap().as_deref(),
            Some("as%20of=2026-01-02%2009:30&note=a%26b%3Dc%2Bd%25e&100%25=%C3%A9")
        );
        let read = url.parameters(true).unwrap();
        assert_eq!(read.get("as of"), Some("2026-01-02 09:30"));
        assert_eq!(read.get("note"), Some("a&b=c+d%e"));
        assert_eq!(read.get("100%"), Some("\u{e9}"));

        // A raw view speaks the query's own bytes and refuses text it cannot carry.
        let raw = url.parameters(false).unwrap();
        assert_eq!(raw.get("as%20of"), Some("2026-01-02%2009:30"));
        let mut raw = raw.into_owned();
        assert!(raw.insert("k", "a b").is_err());
        assert!(raw.insert("k", "a%2").is_err());
        assert!(raw.insert("k", "a&b").is_err());
    }

    /// A failing escape in a query pair names its byte in the query it came from.
    ///
    /// The pairs are a view over one component, so an offset relative to the half
    /// pair a split produced addresses nothing the caller holds.
    #[test]
    fn a_query_pair_escape_failure_names_its_byte_in_the_query() {
        for (query, position, target) in [
            ("alphabeta=%FF", 10, "uri query value"),
            ("a=1&bcd=%FF", 8, "uri query value"),
            ("%FF=1", 0, "uri query key"),
            ("a=1&b=2&%FF=3", 8, "uri query key"),
            ("a=1&b=2&c=%C0%AF", 10, "uri query value"),
        ] {
            let error = yggdryl::Parameters::from_query(query, true)
                .err()
                .unwrap_or_else(|| panic!("{query:?} must not decode"));
            let (reported_target, reported, _) = parse_failure(&error);
            assert_eq!(reported_target, target, "{query:?}");
            assert_eq!(reported, position, "{query:?}");
            assert_eq!(&query[reported..reported + 1], "%", "{query:?}");
        }

        // The same offsets reach a caller through the URI's own view.
        let error = Uri::from_str("https://example.test/?a=1&note=%FF")
            .unwrap()
            .parameters(true)
            .expect_err("the escape must not decode");
        assert_eq!(parse_failure(&error).1, 9);
    }
}

mod value {

    /// The query addressed as its pairs.
    mod parameters {
        use yggdryl::Parameters;
        use yggdryl::{Uri, Url};

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
                Parameters::from_query("symbol=AAPL&venue=XNAS&symbol=MSFT", false).unwrap();

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
                parameters.into_query().as_deref(),
                Some("venue=XNAS&region=us")
            );

            parameters.clear();
            assert_eq!(parameters.into_query(), None);
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
        fn a_raw_view_takes_back_every_value_it_answered_with() {
            // The split takes the first `=`, so a value may hold more of them and
            // still read back as it was written. What this view answers with, it
            // must accept: `get` then `insert` is a round trip, not a refusal.
            let mut parameters = Parameters::from_query("filter=a=b&plain=1", false).unwrap();
            assert_eq!(parameters.get("filter"), Some("a=b"));

            let held = parameters.get("filter").unwrap().to_owned();
            parameters.insert("filter", &held).unwrap();
            parameters.append("copy", &held).unwrap();
            assert_eq!(
                parameters.into_query().as_deref(),
                Some("filter=a=b&plain=1&copy=a=b")
            );
            assert_eq!(
                Parameters::from_query("filter=a=b&plain=1&copy=a=b", false)
                    .unwrap()
                    .get_all("filter")
                    .collect::<Vec<_>>(),
                ["a=b"]
            );

            // A key still cannot carry what ends it.
            assert!(parameters.insert("a=b", "1").is_err());
        }

        #[test]
        fn a_raw_view_refuses_text_the_query_syntax_cannot_carry() {
            let mut parameters = Parameters::from_query("symbol=AAPL", false).unwrap();

            for (key, value) in [("as of", "x"), ("note", "a&b"), ("a=b", "1")] {
                let error = parameters.insert(key, value).unwrap_err().to_string();
                assert!(!error.is_empty(), "{key}={value}");
            }
            assert_eq!(parameters.into_query().as_deref(), Some("symbol=AAPL"));

            // The same text is accepted by a decoding view, which encodes it.
            let mut decoded = Parameters::from_query("symbol=AAPL", true).unwrap();
            decoded.insert("as of", "a&b").unwrap();
            assert_eq!(
                decoded.into_query().as_deref(),
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
            url.set_parameters(&Parameters::new(false)).unwrap();
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
            assert_eq!(edited.into_query().as_deref(), Some("ab=3"));
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
}
