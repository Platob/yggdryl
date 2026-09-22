//! `rust/src/uri/urn.rs`: uniform resource names and the escapes their
//! namespace-specific strings keep.

mod encoding {

    use yggdryl::Urn;

    /// RFC 8141 s2 and s3.1: the NSS carries escapes, and they are never decoded
    /// for comparison, so `%2C` and `,` stay two different URNs.
    #[test]
    fn urn_namespace_specific_strings_keep_their_escapes() {
        let urn = Urn::from_str("URN:EXAMPLE:a123%2cz456").unwrap();

        // Scheme and namespace lowercase; the NSS keeps its case and its escapes,
        // whose hex digits uppercase.
        assert_eq!(urn.to_string(), "urn:example:a123%2Cz456");
        assert_eq!(urn.namespace(), "example");
        assert_eq!(urn.namespace_specific(), "a123%2Cz456");
        assert_ne!(urn, Urn::from_str("urn:example:a123,z456").unwrap());
        assert_ne!(urn, Urn::from_str("urn:example:A123%2Cz456").unwrap());

        // The NSS may hold `/`, and `?`, `#`, `[`, `]` only as escapes.
        assert_eq!(
            Urn::from_str("urn:example:1/406/47452/2")
                .unwrap()
                .file_name(),
            Some("2")
        );
        let escaped = Urn::from_str("urn:example:a%5Bb%5D%3Fc").unwrap();
        assert_eq!(escaped.namespace_specific(), "a%5Bb%5D%3Fc");
        assert_eq!(escaped.path_text(true).unwrap(), "example:a[b]?c");

        // A `?` that opens neither an r- nor a q-component is a syntax error.
        assert!(Urn::from_str("urn:example:foo?bar").is_err());
        let resolved = Urn::from_str("urn:example:foo?+CCResolve:cc=uk?=q1#f1").unwrap();
        assert_eq!(
            resolved.query(false).unwrap().as_deref(),
            Some("+CCResolve:cc=uk?=q1")
        );
        assert_eq!(resolved.fragment(false).unwrap().as_deref(), Some("f1"));
    }
}

mod location {

    use yggdryl::{Uri, Url, Urn};

    /// A name spells a path: the namespace leads it and every `:` in the
    /// namespace-specific string is a separator, so one base turns a whole
    /// namespace of names into locations.
    #[test]
    fn a_name_spells_a_path_and_resolves_under_a_base() {
        let urn = Urn::from_str("urn:lake:trades:2026:part.parquet").unwrap();

        assert_eq!(
            urn.locator_path().unwrap().as_str(),
            "lake/trades/2026/part.parquet"
        );
        assert_eq!(
            urn.resolve(&Url::from_str("s3://market-data/warehouse/").unwrap())
                .unwrap()
                .to_string(),
            "s3://market-data/warehouse/lake/trades/2026/part.parquet"
        );

        // A name with no interior separators is one segment under its namespace.
        assert_eq!(
            Urn::from_str("urn:isbn:9780141036144")
                .unwrap()
                .locator_path()
                .unwrap()
                .as_str(),
            "isbn/9780141036144"
        );

        // Escapes cross as the name holds them: a `%2F` stays one name, never a
        // second segment.
        let escaped = Urn::from_str("urn:example:a%2Fb").unwrap();
        assert_eq!(escaped.locator_path().unwrap().as_str(), "example/a%2Fb");

        // An empty part would let two names spell one path, so it is refused.
        let error = Urn::from_str("urn:example:a::b")
            .unwrap()
            .locator_path()
            .expect_err("an empty part spells no path");
        assert!(error.to_string().contains("empty name part"));
    }

    /// With no base named, a name resolves under the process working directory,
    /// which is the root every relative path is read against.
    #[test]
    fn a_name_with_no_base_resolves_under_the_working_directory() {
        let urn = Urn::from_str("urn:lake:trades:part.parquet").unwrap();
        let located = urn.locator().unwrap();

        assert!(located.is_local());
        assert!(
            located.to_string().ends_with("/lake/trades/part.parquet"),
            "{located}"
        );
        assert_eq!(located.file_name(), Some("part.parquet"));

        // The directory crosses as the platform path it is and the name as the
        // URI text it is, so an escape the name carries is not encoded twice.
        assert!(
            Urn::from_str("urn:example:a%2Fb")
                .unwrap()
                .locator()
                .unwrap()
                .to_string()
                .ends_with("/example/a%2Fb")
        );

        // The whole identifier answers the same location its narrowing does.
        assert_eq!(
            Uri::from_str("urn:lake:trades:part.parquet")
                .unwrap()
                .locator()
                .unwrap(),
            located
        );
    }
}
